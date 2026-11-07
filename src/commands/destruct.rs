use anyhow::{Context, Result};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use crate::{config, platform};

fn confirm(root: &Path) -> Result<bool> {
    println!(
        "Remove all binage data at \"{}\" and its PATH entry?",
        root.display()
    );

    print!("Continue? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim(), "y" | "Y"))
}

/// Try to restore the PATH entry if the uninstall operation fails.
fn restore_path(bin_dir: &Path, error: anyhow::Error) -> anyhow::Error {
    match platform::add_to_path(bin_dir) {
        Ok(_) => error.context("Uninstall failed. The PATH entry was restored."),
        Err(restore_error) => error.context(format!(
            "Uninstall failed, and restoring the PATH entry also failed: {restore_error:#}"
        )),
    }
}

#[cfg(unix)]
fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Cannot remove {}", path.display())),
    }
}

pub fn run() -> Result<()> {
    let root = match config::configured_root()? {
        Some(path) => path,
        None => config::default_root()?,
    };

    if !root.join(config::MANIFEST_FILE).is_file() {
        anyhow::bail!("binage is not initialized at {}.", root.display());
    }

    let bin_dir = root.join("bin");

    if !bin_dir.is_dir() || !root.join("packages").is_dir() {
        anyhow::bail!("Invalid binage installation directory: {}", root.display());
    }

    let pointer = config::root_pointer_path()?;

    if !confirm(&root)? {
        println!("Cancelled.");
        return Ok(());
    }

    // Remove only the PATH configuration owned by binage.
    platform::remove_from_path(&bin_dir)?;

    #[cfg(windows)]
    {
        if let Err(error) = schedule_windows_cleanup(&root, &pointer) {
            return Err(restore_path(&bin_dir, error));
        }

        println!("Uninstall scheduled. binage will be removed after this process exits.");
    }

    #[cfg(unix)]
    {
        // Remember whether a pointer existed so it can be restored
        // if removing the installation directory fails.
        let pointer_existed = match fs::symlink_metadata(&pointer) {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(restore_path(&bin_dir, error.into()));
            }
        };

        // Remove the pointer before deleting the installation directory.
        if let Err(error) = remove_file_if_present(&pointer) {
            return Err(restore_path(&bin_dir, error));
        }

        if let Err(error) =
            fs::remove_dir_all(&root).with_context(|| format!("Cannot remove {}", root.display()))
        {
            // Restore the pointer if it existed before uninstall.
            if pointer_existed {
                if let Err(pointer_error) = config::save_root_pointer(&root) {
                    let error = error.context(format!(
                        "Additionally failed to restore the installation-root pointer: {pointer_error:#}"
                    ));

                    return Err(restore_path(&bin_dir, error));
                }
            }

            return Err(restore_path(&bin_dir, error));
        }

        println!("binage removed.");
    }

    Ok(())
}

#[cfg(windows)]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn schedule_windows_cleanup(root: &Path, pointer: &Path) -> Result<()> {
    use std::process::Command;

    let script_path =
        std::env::temp_dir().join(format!("binage-uninstall-{}.ps1", std::process::id()));

    let script = format!(
        r#"
$targetPid = {}
while (Get-Process -Id $targetPid -ErrorAction SilentlyContinue) {{
    Start-Sleep -Milliseconds 250
}}

Remove-Item -LiteralPath {} -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue

$configDir = Split-Path -Parent {}
if (Test-Path -LiteralPath $configDir) {{
    if (-not (Get-ChildItem -LiteralPath $configDir -Force | Select-Object -First 1)) {{
        Remove-Item -LiteralPath $configDir -Force -ErrorAction SilentlyContinue
    }}
}}

Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue
"#,
        std::process::id(),
        ps_quote(&root.to_string_lossy()),
        ps_quote(&pointer.to_string_lossy()),
        ps_quote(&pointer.to_string_lossy()),
        ps_quote(&script_path.to_string_lossy()),
    );

    // Write UTF-16LE with BOM so Windows PowerShell handles Unicode paths.
    let mut bytes = vec![0xFF, 0xFE];

    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }

    fs::write(&script_path, bytes)?;

    if let Err(error) = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .spawn()
    {
        let _ = fs::remove_file(&script_path);
        return Err(error).context("Cannot schedule binage cleanup.");
    }

    Ok(())
}
