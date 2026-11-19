use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    // `ncp_switch` embeds an ST67 `.ota` image. ST's binaries are licensed
    // separately and never live in this repo, so the image is named at build
    // time through `ST67_OTA_IMAGE` (scripts/ncp_switch.sh sets it). Without
    // it, `IMAGE` is empty and every other bin builds exactly as before.
    println!("cargo:rerun-if-env-changed=ST67_OTA_IMAGE");
    let generated = match env::var("ST67_OTA_IMAGE") {
        Ok(path) if !path.is_empty() => {
            let abs = fs::canonicalize(&path)
                .map_err(|e| format!("ST67_OTA_IMAGE={path}: {e}"))?;
            println!("cargo:rerun-if-changed={}", abs.display());
            let abs = abs.to_str().ok_or("ST67_OTA_IMAGE is not valid UTF-8")?;
            let name = abs.rsplit('/').next().unwrap_or(abs);
            // Which profile the image installs, from ST's file name
            // (`st67w611m_mission_t01_v2.0.97.bin.ota`).
            let target = if name.contains("mission_t01") {
                "t01"
            } else if name.contains("mission_t02") {
                "t02"
            } else {
                return Err(format!("ST67_OTA_IMAGE={name} is neither a mission_t01 nor a mission_t02 image").into());
            };
            format!(
                "pub static IMAGE: &[u8] = include_bytes!({abs:?});\n\
                 pub static IMAGE_NAME: &str = {name:?};\n\
                 pub static TARGET: &str = {target:?};\n"
            )
        }
        _ => "pub static IMAGE: &[u8] = &[];\npub static IMAGE_NAME: &str = \"\";\npub static TARGET: &str = \"\";\n".into(),
    };
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("ota_image.rs"),
        generated,
    )?;

    Ok(())
}
