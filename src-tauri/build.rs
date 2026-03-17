use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if cfg!(target_os = "windows") {
        stage_webview2_loader().expect("failed to stage WebView2Loader.dll");
    }

    tauri_build::build();
}

fn stage_webview2_loader() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .ok_or("failed to resolve target profile dir from OUT_DIR")?;

    let target = env::var("TARGET")?;
    let arch_dir = if target.contains("x86_64") {
        "x64"
    } else if target.contains("aarch64") {
        "arm64"
    } else if target.contains("i686") {
        "x86"
    } else {
        return Err(format!("unsupported windows target: {target}").into());
    };

    let source = find_webview2_loader(profile_dir, arch_dir)
        .ok_or("unable to locate WebView2Loader.dll in cargo build output")?;
    let staged = profile_dir.join("WebView2Loader.dll");
    fs::copy(&source, &staged)?;

    println!("cargo:rerun-if-changed={}", source.display());
    println!(
        "cargo:warning=staged WebView2Loader.dll from {}",
        source.display()
    );

    Ok(())
}

fn find_webview2_loader(profile_dir: &Path, arch_dir: &str) -> Option<PathBuf> {
    let build_dir = profile_dir.join("build");
    let entries = fs::read_dir(build_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy();
        if !name.starts_with("webview2-com-sys-") {
            continue;
        }

        let candidate = path.join("out").join(arch_dir).join("WebView2Loader.dll");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
