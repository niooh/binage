#[cfg(windows)]
#[path = "format.rs"]
mod format;

#[cfg(windows)]
fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("binage shim: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn run() -> Result<i32, Box<dyn std::error::Error>> {
    use std::{env, process::Command};

    // Locate this shim executable.
    let shim_path = env::current_exe()?;

    let relative_target = format::read_target(&shim_path)?;

    // bin/<entry>.exe -> bin -> binage root.
    let bin_dir = shim_path.parent().ok_or("shim has no parent directory")?;

    let root = bin_dir.parent().ok_or("cannot determine binage root")?;

    let target = root.join(relative_target);

    if !target.is_file() {
        return Err(format!("target executable does not exist: {}", target.display()).into());
    }

    // Forward arguments and inherit standard streams.
    let status = Command::new(&target)
        .args(env::args_os().skip(1))
        .status()?;

    Ok(status.code().unwrap_or(1))
}
