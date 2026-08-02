use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=migrations-sqlite");
    println!("cargo:rerun-if-env-changed=VIFU_CONSOLE_ASSETS_DIR");
    println!("cargo:rerun-if-env-changed=VIFU_REQUIRE_CONSOLE_ASSETS");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let target = out_dir.join("console_assets.rs");
    let asset_dir = env::var_os("VIFU_CONSOLE_ASSETS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"))
                .join("../../target/vifu-console-assets")
        });
    println!("cargo:rerun-if-changed={}", asset_dir.display());

    if !asset_dir.join("index.html").is_file() {
        if env::var_os("VIFU_REQUIRE_CONSOLE_ASSETS").is_some() {
            panic!(
                "embedded Dashboard assets are required but {} does not contain index.html; run `bun run build:console` before building Vifu",
                asset_dir.display()
            );
        }
        write_fallback(&target).expect("write fallback console assets");
        return;
    }

    let mut files = Vec::new();
    collect_files(&asset_dir, &asset_dir, &mut files).expect("collect console assets");
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut source = String::from(
        "pub struct ConsoleAsset {\n    pub path: &'static str,\n    pub content_type: &'static str,\n    pub bytes: &'static [u8],\n}\n\n",
    );
    source.push_str("pub const CONSOLE_ASSETS_AVAILABLE: bool = true;\n\n");
    source.push_str("pub static CONSOLE_ASSETS: &[ConsoleAsset] = &[\n");
    for (relative, absolute) in files {
        println!("cargo:rerun-if-changed={}", absolute.display());
        source.push_str("    ConsoleAsset {\n");
        source.push_str(&format!("        path: \"{}\",\n", escape(&relative)));
        source.push_str(&format!(
            "        content_type: \"{}\",\n",
            content_type(&relative)
        ));
        source.push_str(&format!(
            "        bytes: include_bytes!(r#\"{}\"#),\n",
            absolute.display()
        ));
        source.push_str("    },\n");
    }
    source.push_str("];\n");
    fs::write(target, source).expect("write console asset module");
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("asset file is under root")
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, path));
        }
    }
    Ok(())
}

fn write_fallback(target: &Path) -> io::Result<()> {
    fs::write(
        target,
        r#"pub struct ConsoleAsset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub const CONSOLE_ASSETS_AVAILABLE: bool = false;

pub static CONSOLE_ASSETS: &[ConsoleAsset] = &[
    ConsoleAsset {
        path: "index.html",
        content_type: "text/html; charset=utf-8",
        bytes: b"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Vifu Console</title></head><body><main><h1>Vifu Console assets are not bundled.</h1><p>Run `bun run build:console` before building the release binary.</p></main></body></html>",
    },
];
"#,
    )
}

fn content_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
