use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    config,
    manifest::{BinaryRecord, Manifest},
    platform, util,
};

static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn run(path: Option<PathBuf>) -> Result<()> {
    // The first successful initialization fixes the installation root.
    // Later `init` calls never relocate it, because the PATH entry is immutable.
    let root = match config::configured_root()? {
        Some(existing) => existing,
        None => config::prepare_root(path)?,
    };

    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir)?;
    fs::create_dir_all(root.join("packages"))?;

    let manifest_path = root.join(config::MANIFEST_FILE);
    let mut manifest = Manifest::load(&manifest_path)?;

    // Install and register binage itself.
    // Declining an overwrite does not prevent the rest of init from completing.
    let installed = install_self(&root, &bin_dir, &manifest_path, &mut manifest)?;

    // Save the manifest even when the user declined an overwrite.
    // In that case, existing records remain unchanged.
    if !installed {
        manifest.save(&manifest_path)?;
    }

    config::save_root_pointer(&root)?;

    let changed = platform::add_to_path(&bin_dir)?;

    println!("binage initialized at {}", root.display());

    if installed {
        println!("Registered binage in the manifest.");
    } else {
        println!("The existing binage executable was left unchanged.");
    }

    if !changed {
        println!("PATH was already configured; it was left unchanged.");
    }

    Ok(())
}

/// Install the currently running executable into the managed bin directory.
///
/// Returns false when the user declines to overwrite an existing executable.
fn install_self(
    root: &Path,
    bin_dir: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
) -> Result<bool> {
    let source = std::env::current_exe().context("cannot determine the current executable path")?;

    let source = fs::canonicalize(&source).context("cannot resolve the current executable path")?;

    let file_name = source
        .file_name()
        .context("current executable has no filename")?
        .to_string_lossy()
        .into_owned();

    let name = util::command_name(&source)?;

    if name != "binage" {
        bail!("unexpected executable name `{name}`; expected `binage`");
    }

    // Do not allow binage to take over a package entry with the same name.
    if manifest
        .packages
        .values()
        .any(|package| package.entries.contains_key(&name))
    {
        bail!("cannot register `{name}`: the command name belongs to an installed package");
    }

    let destination = bin_dir.join(&file_name);

    let destination_exists = match fs::symlink_metadata(&destination) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!(
                    "refusing to overwrite a path that is not a regular file: {}",
                    destination.display()
                );
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };

    // If binage is already running from its managed location, there is
    // nothing to copy and no overwrite confirmation is necessary.
    let already_installed = if destination_exists {
        fs::canonicalize(&destination)
            .map(|path| path == source)
            .unwrap_or(false)
    } else {
        false
    };

    if destination_exists && !already_installed {
        println!(
            "The executable already exists:\n  {}",
            destination.display()
        );

        if !confirm_overwrite("Overwrite the existing binage executable?")? {
            println!("Keeping the existing executable.");
            return Ok(false);
        }
    }

    let backup = if destination_exists && !already_installed {
        Some(unique_path(bin_dir, ".binage-backup-", "")?)
    } else {
        None
    };

    let temporary = if already_installed {
        None
    } else {
        Some(unique_path(bin_dir, ".binage-install-", ".tmp")?)
    };

    // Copy to a temporary file first. The existing executable is not
    // touched unless the copy succeeds.
    if let Some(temp) = &temporary {
        if let Err(error) = util::copy_file(&source, temp) {
            let _ = fs::remove_file(temp);
            return Err(error).context("failed to stage the binage executable");
        }
    }

    // Move the previous executable aside before installing the new one.
    if let (Some(backup_path), true) = (&backup, destination_exists) {
        if let Err(error) = fs::rename(&destination, backup_path) {
            if let Some(temp) = &temporary {
                let _ = fs::remove_file(temp);
            }

            return Err(error).with_context(|| {
                format!(
                    "cannot back up the existing executable: {}",
                    destination.display()
                )
            });
        }
    }

    // Move the staged executable into its final location.
    if let Some(temp) = &temporary {
        if let Err(error) = fs::rename(temp, &destination) {
            let _ = fs::remove_file(temp);

            if let Some(backup_path) = &backup {
                if let Err(restore_error) = fs::rename(backup_path, &destination) {
                    bail!(
                        "installation failed: {error}; additionally failed to restore the previous executable: {restore_error}"
                    );
                }
            }

            return Err(error).with_context(|| {
                format!("cannot install executable at {}", destination.display())
            });
        }
    }

    // Register the executable only after the filesystem operation succeeds.
    manifest.binaries.insert(name, BinaryRecord { file_name });

    if let Err(error) = manifest.save(manifest_path) {
        // Roll back the executable if manifest persistence fails.
        if !already_installed {
            let _ = fs::remove_file(&destination);
        }

        if let Some(backup_path) = &backup {
            if let Err(restore_error) = fs::rename(backup_path, &destination) {
                bail!(
                    "manifest update failed: {error}; additionally failed to restore the previous executable: {restore_error}"
                );
            }
        }

        return Err(error).context("failed to update the binage manifest");
    }

    // The new executable and manifest are now in place.
    if let Some(backup_path) = &backup {
        let _ = fs::remove_file(backup_path);
    }

    println!("Installed executable: {}", destination.display());

    let _ = root;
    Ok(true)
}

/// Ask for explicit confirmation. Only y/Y accepts the overwrite.
fn confirm_overwrite(message: &str) -> Result<bool> {
    print!("{message} [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim(), "y" | "Y"))
}

/// Generate an unused path in the specified directory.
fn unique_path(dir: &Path, prefix: &str, suffix: &str) -> Result<PathBuf> {
    loop {
        let id = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            "{prefix}{}{suffix}",
            format!("{}-{id}", std::process::id())
        ));

        match fs::symlink_metadata(&path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(path);
            }
            Err(error) => return Err(error.into()),
        }
    }
}
