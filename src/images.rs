use std::io::Cursor;

use exif::{In, Reader, Tag};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, Rgb, RgbImage};
use jpeg_decoder::{Decoder, PixelFormat};

use crate::error::AppErr;
use crate::util::MAX_PIXELS;

const MAX_SIDE: u32 = 2048;
const THUMB_SIDE: u32 = 360;
const JPEG_QUALITY: u8 = 80;
const THUMB_QUALITY: u8 = 70;

pub struct ProcessedImage {
    pub full: Vec<u8>,
    pub thumb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn process(bytes: &[u8]) -> Result<ProcessedImage, AppErr> {
    let oriented = decode(bytes)?;
    let full = fit(&oriented, MAX_SIDE);
    let width = full.width();
    let height = full.height();
    if width == 0 || height == 0 {
        return Err(AppErr::BadRequest("无法处理这张图片"));
    }
    let thumb = fit(&full, THUMB_SIDE);
    let full_jpeg = encode(&full, JPEG_QUALITY)?;
    let thumb_jpeg = encode(&thumb, THUMB_QUALITY)?;
    if !is_jpeg(&full_jpeg)
        || !is_jpeg(&thumb_jpeg)
        || has_exif_marker(&full_jpeg)
        || has_exif_marker(&thumb_jpeg)
    {
        return Err(AppErr::Internal);
    }
    Ok(ProcessedImage {
        full: full_jpeg,
        thumb: thumb_jpeg,
        width,
        height,
    })
}

pub fn pixels_allowed(width: u32, height: u32) -> bool {
    (width as u64).saturating_mul(height as u64) <= MAX_PIXELS
        && width > 0
        && height > 0
        && width <= 20_000
        && height <= 20_000
}

fn decode(bytes: &[u8]) -> Result<DynamicImage, AppErr> {
    let format = image::guess_format(bytes)
        .map_err(|_| AppErr::BadRequest("只支持 JPEG、PNG、WebP 和 GIF"))?;
    let image = match format {
        ImageFormat::Jpeg => decode_jpeg(bytes)?,
        ImageFormat::Png => decode_limited(
            image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(img_err)?,
        )?,
        ImageFormat::WebP => decode_limited(
            image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(img_err)?,
        )?,
        ImageFormat::Gif => decode_limited(
            image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(img_err)?,
        )?,
        _ => return Err(AppErr::BadRequest("只支持 JPEG、PNG、WebP 和 GIF")),
    };
    Ok(apply_orientation(image, orientation(bytes)))
}

fn img_err(_: image::ImageError) -> AppErr {
    AppErr::BadRequest("无法处理这张图片")
}

fn decode_limited<D>(decoder: D) -> Result<DynamicImage, AppErr>
where
    D: ImageDecoder,
{
    let (width, height) = decoder.dimensions();
    if !pixels_allowed(width, height) {
        return Err(AppErr::BadRequest("图片尺寸过大"));
    }
    DynamicImage::from_decoder(decoder).map_err(|_| AppErr::BadRequest("无法处理这张图片"))
}

fn decode_jpeg(bytes: &[u8]) -> Result<DynamicImage, AppErr> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(80 * 1024 * 1024);
    if decoder.scale(MAX_SIDE as u16, MAX_SIDE as u16).is_err() {
        return Err(AppErr::BadRequest("无法处理这张图片"));
    }
    let pixels = decoder
        .decode()
        .map_err(|_| AppErr::BadRequest("无法处理这张图片"))?;
    let info = decoder
        .info()
        .ok_or(AppErr::BadRequest("无法处理这张图片"))?;
    if !pixels_allowed(info.width as u32, info.height as u32) {
        return Err(AppErr::BadRequest("图片尺寸过大"));
    }
    let rgb = match info.pixel_format {
        PixelFormat::RGB24 => RgbImage::from_raw(info.width as u32, info.height as u32, pixels)
            .ok_or(AppErr::BadRequest("无法处理这张图片"))?,
        PixelFormat::L8 => {
            let mut rgb = RgbImage::new(info.width as u32, info.height as u32);
            for (pixel, value) in rgb.pixels_mut().zip(pixels.iter()) {
                *pixel = Rgb([*value, *value, *value]);
            }
            rgb
        }
        PixelFormat::CMYK32 => {
            let mut rgb = RgbImage::new(info.width as u32, info.height as u32);
            for (pixel, chunk) in rgb.pixels_mut().zip(pixels.chunks_exact(4)) {
                *pixel = Rgb(cmyk_to_rgb(chunk[0], chunk[1], chunk[2], chunk[3]));
            }
            rgb
        }
        PixelFormat::L16 => return Err(AppErr::BadRequest("无法处理这张图片")),
    };
    Ok(DynamicImage::ImageRgb8(rgb))
}

fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> [u8; 3] {
    let mix = |channel: u8| {
        let value = 255u32 - (channel as u32 * (255 - k as u32) / 255 + k as u32);
        value.min(255) as u8
    };
    [mix(c), mix(m), mix(y)]
}

fn orientation(bytes: &[u8]) -> u32 {
    let mut cursor = Cursor::new(bytes);
    let Ok(exif) = Reader::new().read_from_container(&mut cursor) else {
        return 1;
    };
    exif.get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .unwrap_or(1)
}

fn apply_orientation(image: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

fn fit(image: &DynamicImage, max_side: u32) -> DynamicImage {
    let width = image.width();
    let height = image.height();
    let longest = width.max(height);
    if longest <= max_side {
        return image.clone();
    }
    let scale = max_side as f32 / longest as f32;
    let new_width = ((width as f32 * scale).round() as u32).max(1);
    let new_height = ((height as f32 * scale).round() as u32).max(1);
    image.resize(new_width, new_height, FilterType::Triangle)
}

fn encode(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, AppErr> {
    let rgb = flatten(image);
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|_| AppErr::Internal)?;
    Ok(out)
}

fn flatten(image: &DynamicImage) -> RgbImage {
    if let DynamicImage::ImageRgb8(rgb) = image {
        return rgb.clone();
    }
    let rgba = image.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let blend = |channel: u8| ((channel as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        out.put_pixel(
            x,
            y,
            Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])]),
        );
    }
    out
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes[0] == 0xff && bytes[1] == 0xd8
}

pub fn has_exif_marker(bytes: &[u8]) -> bool {
    bytes.windows(6).any(|window| window == b"Exif\0\0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::Reader;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn recompresses_and_drops_exif() {
        let image: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(3000, 1600, |x, y| {
            Rgb([(x * 13 + y * 7) as u8, (x * 3) as u8, (y * 5) as u8])
        });
        let mut raw = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut raw, 95);
        encoder
            .encode(image.as_raw(), 3000, 1600, ExtendedColorType::Rgb8)
            .unwrap();
        let tagged = inject_exif(&raw);
        assert!(has_exif_marker(&tagged));
        let processed = process(&tagged).unwrap();
        assert!(processed.width <= MAX_SIDE && processed.height <= MAX_SIDE);
        assert!(processed.height > processed.width);
        assert!(processed.full.len() < tagged.len());
        assert!(!has_exif_marker(&processed.full));
        assert!(Reader::new()
            .read_from_container(&mut Cursor::new(processed.full.as_slice()))
            .is_err());
        assert_ne!(processed.full, tagged);
    }

    #[test]
    fn rejects_too_many_pixels() {
        assert!(!pixels_allowed(20_000, 20_000));
        let png = png_header(20_000, 20_000);
        assert!(process(&png).is_err());
    }

    fn inject_exif(jpeg: &[u8]) -> Vec<u8> {
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0".to_vec();
        exif.extend_from_slice(&2u16.to_le_bytes());
        exif.extend_from_slice(&0x0112u16.to_le_bytes());
        exif.extend_from_slice(&3u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&6u16.to_le_bytes());
        exif.extend_from_slice(&0u16.to_le_bytes());
        exif.extend_from_slice(&0x8825u16.to_le_bytes());
        exif.extend_from_slice(&4u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        let gps_offset = 8 + 2 + 12 * 2 + 4;
        exif.extend_from_slice(&(gps_offset as u32).to_le_bytes());
        exif.extend_from_slice(&0u32.to_le_bytes());
        exif.extend_from_slice(&1u16.to_le_bytes());
        exif.extend_from_slice(&0x0002u16.to_le_bytes());
        exif.extend_from_slice(&5u16.to_le_bytes());
        exif.extend_from_slice(&3u32.to_le_bytes());
        let value_offset = gps_offset + 2 + 12 + 4;
        exif.extend_from_slice(&(value_offset as u32).to_le_bytes());
        exif.extend_from_slice(&0u32.to_le_bytes());
        for number in [30u32, 1, 0, 1, 0, 1] {
            exif.extend_from_slice(&number.to_le_bytes());
        }
        let mut app1 = Vec::new();
        app1.extend_from_slice(&[0xff, 0xe1]);
        app1.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(&exif);
        let mut out = Vec::with_capacity(2 + app1.len() + jpeg.len() - 2);
        out.extend_from_slice(&jpeg[..2]);
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(tag);
            out.extend_from_slice(data);
            let mut crc = crc32(tag);
            crc = crc32_update(crc, data);
            out.extend_from_slice(&crc.to_be_bytes());
            out
        }
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &ihdr));
        png
    }

    fn crc32(data: &[u8]) -> u32 {
        crc32_update(0xffffffff, data) ^ 0xffffffff
    }

    fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb88320 & mask);
            }
        }
        crc
    }
}
