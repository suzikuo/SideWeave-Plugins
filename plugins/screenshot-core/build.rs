use std::env;
use std::path::PathBuf;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("windows.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!(
        "cargo:rustc-link-arg-bin=sideweave_screenshot-core=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bin=sideweave_screenshot-core=/MANIFEST:EMBED");
}
