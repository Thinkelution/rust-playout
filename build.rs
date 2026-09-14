use std::{env, fs, path::Path};

fn collect(root: &Path, directory: &Path, code: &mut String) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, code);
        } else {
            let key = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let mime = match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
                "html" => "text/html; charset=utf-8",
                "js" => "text/javascript; charset=utf-8",
                "css" => "text/css; charset=utf-8",
                "svg" => "image/svg+xml",
                "png" => "image/png",
                "ico" => "image/x-icon",
                _ => "application/octet-stream",
            };
            code.push_str(&format!(
                "{key:?} => Some(({mime:?}, include_bytes!({:?}))),\n",
                path.to_str().unwrap()
            ));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=web/dist");
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("web/dist");
    assert!(
        root.join("index.html").exists(),
        "Build the embedded control room first: cd web && npm ci && npm run build"
    );
    let mut code = String::from(
        "pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> { match path {\n",
    );
    collect(&root, &root, &mut code);
    code.push_str("_ => None, } }\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("web_assets.rs"),
        code,
    )
    .unwrap();
}
