use std::{
    env,
    path::PathBuf,
    process::Command,
};

fn main() {
	println!("cargo:rerun-if-changed=src/shim/main.rs");
    println!("cargo:rerun-if-changed=src/shim/format.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS")
        .expect("CARGO_CFG_TARGET_OS is not set");

    // Unix does not use the Windows shim.
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").unwrap()
    );

    let out_dir = PathBuf::from(
        env::var("OUT_DIR").unwrap()
    );

    let target = env::var("TARGET").unwrap();
    let rustc = env::var("RUSTC").unwrap();

    let source = manifest_dir.join("src/shim/main.rs");
    let output = out_dir.join("binage-shim.exe");

    let mut command = Command::new(rustc);

    command
        .arg("--crate-name")
        .arg("binage_shim")
        .arg("--crate-type")
        .arg("bin")
		.args([
			"-C", "opt-level=3",
			"-C", "panic=abort",
			"-C", "codegen-units=1",
			"-C", "strip=symbols",
			"-C", "lto=fat",
		])
        .arg("--edition=2021")
        .arg("--target")
        .arg(&target)
        .arg(&source)
        .arg("-o")
        .arg(&output);

    // Match the Cargo build profile.
    if env::var("PROFILE").as_deref() == Ok("release") {
        command.arg("-C").arg("opt-level=3");
    } else {
        command.arg("-C").arg("opt-level=0");
    }

    let status = command
        .status()
        .expect("failed to execute rustc for shim");

    if !status.success() {
        panic!("failed to compile binage shim");
    }

    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=TARGET");
	println!(
		"cargo:rustc-env=BINAGE_SHIM_PATH={}",
		output.display()
	);
}
