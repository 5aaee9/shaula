use std::{env, error::Error, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../web");
    for input in [
        "src",
        "public",
        "index.html",
        "vite.config.ts",
        "dev-auth.ts",
        "tsconfig.json",
        "package.json",
        "package-lock.json",
        "node_modules/.package-lock.json",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    println!("cargo:rerun-if-env-changed=NODE");
    let vite = root.join("node_modules/vite/bin/vite.js");
    if !vite.is_file() {
        return Err(
            "web dependencies missing: run `npm ci --prefix web` before building Shaula".into(),
        );
    }
    let status = Command::new(env::var_os("NODE").unwrap_or_else(|| "node".into()))
        .arg(&vite)
        .arg("build")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("Vite build failed; the binary was not built with stale web assets".into());
    }
    Ok(())
}
