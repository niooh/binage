use anyhow::{bail, Context, Result};
use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

/// Return the command name for an executable entry.
///
/// Only .exe is treated as a platform executable suffix.
/// Other extensions are preserved.
pub fn command_name(path: &Path) -> Result<String> {
    let file_name = path
        .file_name()
        .context("entry has no filename")?
        .to_string_lossy()
        .into_owned();

    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
    {
        bail!("invalid entry name");
    }

    let name = if file_name
        .to_ascii_lowercase()
        .ends_with(".exe")
    {
        file_name[..file_name.len() - 4].to_string()
    } else {
        file_name
    };

    if name.is_empty() {
        bail!("entry name cannot be empty");
    }

    Ok(name)
}

/// Copy a file without overwriting an existing destination.
pub fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;

    if !metadata.file_type().is_file() {
        bail!("source is not a regular file: {}", source.display());
    }

    let result = (|| -> Result<()> {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .with_context(|| {
                format!(
                    "destination already exists or cannot be created: {}",
                    destination.display()
                )
            })?;

        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);

        fs::set_permissions(destination, metadata.permissions())?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(destination);
    }

    result
}

/// Recursively copy a directory without overwriting.
pub fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;

    if !source_metadata.file_type().is_dir() {
        bail!("source is not a directory");
    }

    fs::create_dir(destination)?;

    let result = (|| -> Result<()> {
        for item in fs::read_dir(source)? {
            let item = item?;
            let from = item.path();
            let to = destination.join(item.file_name());

            let metadata = fs::symlink_metadata(&from)?;
            let file_type = metadata.file_type();

            if file_type.is_symlink() {
                bail!("symbolic links are not supported: {}", from.display());
            }

            if file_type.is_dir() {
                copy_dir(&from, &to)?;
            } else if file_type.is_file() {
                copy_file(&from, &to)?;
            } else {
                bail!("unsupported filesystem entry: {}", from.display());
            }
        }

        fs::set_permissions(destination, source_metadata.permissions())?;

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }

    result
}

/// Check whether a path exists, including dangling symlinks.
pub fn path_occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Check Windows command-name conflicts.
///
/// Windows command lookup can resolve .COM, .EXE, .BAT and .CMD
/// even when the extension is omitted.
#[cfg(windows)]
pub fn check_entry_available(bin_dir: &Path, name: &str) -> Result<()> {
    for extension in ["", ".com", ".exe", ".bat", ".cmd"] {
        let candidate = bin_dir.join(format!("{name}{extension}"));

        if path_occupied(&candidate) {
            bail!("entry name is already occupied: {}", candidate.display());
        }
    }

    Ok(())
}

#[cfg(unix)]
pub fn check_entry_available(bin_dir: &Path, name: &str) -> Result<()> {
    let candidate = bin_dir.join(name);

    if path_occupied(&candidate) {
        bail!("entry name is already occupied: {}", candidate.display());
    }

    Ok(())
}

