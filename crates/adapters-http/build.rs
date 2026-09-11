use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let web = manifest.join("../../apps/web");
    println!("cargo:rerun-if-changed={}", web.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        web.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("vite.config.ts").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest
            .join("../../contracts/observer.openapi.yaml")
            .display()
    );

    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web)
        .status()
        .expect("Bun is required to embed the Workengine Web UI; run `bun install` first");
    assert!(status.success(), "Bun failed to build the embedded Web UI");

    let dist = web.join("dist");
    let mut files = Vec::new();
    collect_files(&dist, &dist, &mut files);
    files.sort();
    let generated =
        PathBuf::from(env::var_os("OUT_DIR").expect("output directory")).join("web_assets.rs");
    let mut source = String::from("pub static WEB_ASSETS: &[(&str, &[u8], &str)] = &[\n");
    for file in files {
        let relative = file.strip_prefix(&dist).expect("asset beneath dist");
        let route = format!("/{}", relative.to_string_lossy());
        let mime = match file.extension().and_then(|extension| extension.to_str()) {
            Some("css") => "text/css; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("json") => "application/json",
            Some("html") => "text/html; charset=utf-8",
            _ => "application/octet-stream",
        };
        source.push_str(&format!(
            "(\"{route}\", include_bytes!(r#\"{}\"#), \"{mime}\"),\n",
            file.display()
        ));
    }
    source.push_str("];\n");
    fs::write(generated, source).expect("write embedded asset source");
}

fn collect_files(dir: &std::path::Path, root: &std::path::Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read Web build directory") {
        let path = entry.expect("read Web build entry").path();
        if path.is_dir() {
            collect_files(&path, root, files);
        } else {
            let _ = root;
            files.push(path);
        }
    }
}
