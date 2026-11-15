use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::crypto::{self, Key};
use crate::error::AppErr;

pub fn images_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("images")
}

pub fn safe_name(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn write_encrypted(key: &Key, dir: &Path, name: &str, plain: &[u8]) -> Result<(), AppErr> {
    if !safe_name(name) {
        return Err(AppErr::Internal);
    }
    let encrypted = crypto::seal(key, name.as_bytes(), plain)?;
    let temp = dir.join(format!("{name}.tmp"));
    let path = dir.join(name);
    std::fs::write(&temp, encrypted).map_err(|err| {
        tracing::error!(error = %err, "写入图片失败");
        AppErr::Internal
    })?;
    restrict_file(&temp);
    std::fs::rename(&temp, &path).map_err(|err| {
        tracing::error!(error = %err, "保存图片失败");
        AppErr::Internal
    })?;
    restrict_file(&path);
    Ok(())
}

pub fn read_encrypted(key: &Key, dir: &Path, name: &str) -> Result<Vec<u8>, AppErr> {
    if !safe_name(name) {
        return Err(AppErr::NotFound);
    }
    let bytes = std::fs::read(dir.join(name)).map_err(|_| AppErr::NotFound)?;
    crypto::open(key, name.as_bytes(), &bytes).map_err(|_| AppErr::NotFound)
}

pub fn remove_names(dir: &Path, names: &[String]) {
    for name in names {
        if safe_name(name) || (name.ends_with(".tmp") && name.len() < 80) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

pub fn remove_orphans(dir: &Path, referenced: &std::collections::HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if referenced.contains(&name) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if modified.elapsed().unwrap_or(Duration::from_secs(0)) < Duration::from_secs(120) {
            continue;
        }
        if safe_name(&name) || name.ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = SystemTime::now();
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}
