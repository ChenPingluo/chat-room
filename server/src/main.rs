#![forbid(unsafe_code)]

fn main() {
    if let Err(err) = chat_server::cli::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
