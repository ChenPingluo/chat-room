use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        std::fs::create_dir_all(&dist).expect("create web/dist");
        std::fs::write(
            &index,
            "<!doctype html><meta charset=\"utf-8\"><title>私语</title><p>前端还没有构建。</p>",
        )
        .expect("write placeholder index");
    }
    println!("cargo:rerun-if-changed=../web/dist");
}
