use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    config::{self, MANIFEST_FILE},
    manifest::{BinaryRecord, Manifest, PackageRecord},
    util,
};

pub fn run(source: PathBuf, entries: Vec<PathBuf>) -> Result<()> {
    let root = config::root_from_manifest()?;
    let manifest_path = root.join(MANIFEST_FILE);

    let mut manifest = Manifest::load(&manifest_path)?;

    let source_meta = fs::symlink_metadata(&source)?;

    if source_meta.file_type().is_symlink() {
        bail!("source itself must not be a symbolic link");
    }

    let source = fs::canonicalize(&source)?;

    if source.is_file() {
        return add_binary(&root, &manifest_path, &mut manifest, &source, entries);
    }

    if source.is_dir() {
        return add_package(&root, &manifest_path, &mut manifest, &source, entries);
    }

    bail!("source must be a regular file or directory");
}

fn add_binary(
    root: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
    source: &Path,
    entries: Vec<PathBuf>,
) -> Result<()> {
    if !entries.is_empty() {
        bail!("-e can only be used with a directory");
    }

    let name = util::command_name(source)?;

    if manifest.has_entry(&name) {
        bail!("entry `{name}` is already registered");
    }

    let bin_dir = root.join("bin");

    util::check_entry_available(&bin_dir, &name)?;

    let file_name = source.file_name().context("missing filename")?;
    let destination = bin_dir.join(file_name);

    if util::path_occupied(&destination) {
        bail!("destination already exists: {}", destination.display());
    }

    // Copy the binary. Never remove the original.
    util::copy_file(source, &destination)?;

    // Unix needs an extensionless alias for .exe files.
    #[cfg(unix)]
    if name != file_name.to_string_lossy() {
        use std::os::unix::fs::symlink;

        let alias = bin_dir.join(&name);

        if let Err(error) = symlink(&destination, &alias) {
            let _ = fs::remove_file(&destination);
            return Err(error.into());
        }
    }

    manifest.binaries.insert(
        name.clone(),
        BinaryRecord {
            file_name: file_name.to_string_lossy().into_owned(),
        },
    );

    if let Err(error) = manifest.save(manifest_path) {
        let _ = fs::remove_file(&destination);

        #[cfg(unix)]
        if name != file_name.to_string_lossy() {
            let _ = fs::remove_file(bin_dir.join(&name));
        }

        return Err(error);
    }

    println!("Installed binary: {name}");

    Ok(())
}

fn add_package(
    root: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
    source: &Path,
    entries: Vec<PathBuf>,
) -> Result<()> {
    if entries.is_empty() {
        bail!("directory installs require at least one -e FILE");
    }

    let package_name = source
        .file_name()
        .context("directory has no filename")?
        .to_string_lossy()
        .into_owned();

    if manifest.packages.contains_key(&package_name) {
        bail!("package `{package_name}` is already registered");
    }

    let destination = root.join("packages").join(&package_name);

    if util::path_occupied(&destination) {
        bail!("package directory already exists: {}", destination.display());
    }

    let bin_dir = root.join("bin");

    let mut selected = BTreeMap::new();
    let mut names = Vec::new();

    for entry in entries {
        if entry.is_absolute() {
            bail!("entry paths must be relative to the package directory");
        }

        let candidate = fs::canonicalize(source.join(&entry))
            .with_context(|| format!("entry does not exist: {}", entry.display()))?;

        if !candidate.starts_with(source) {
            bail!("entry escapes the package directory: {}", entry.display());
        }

        if !fs::symlink_metadata(&candidate)?.file_type().is_file() {
            bail!("entry must be a regular file: {}", entry.display());
        }

        let name = util::command_name(&entry)?;

        if manifest.has_entry(&name) || selected.contains_key(&name) {
            bail!("entry name `{name}` is already registered");
        }

        util::check_entry_available(&bin_dir, &name)?;

        let relative = candidate.strip_prefix(source)?.to_path_buf();

        selected.insert(
            name.clone(),
            relative.to_string_lossy().into_owned(),
        );

        names.push((name, relative));
    }

    reject_symlinks(source)?;

    // Copy the complete package directory.
    util::copy_dir(source, &destination)?;

    let mut created_launchers = Vec::new();

    for (name, relative) in &names {
        let target = destination.join(relative);

        if let Err(error) = create_launcher(&bin_dir, name, &target) {
            rollback_launchers(&bin_dir, &created_launchers);
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }

        created_launchers.push(name.clone());
    }

    manifest.packages.insert(
        package_name.clone(),
        PackageRecord {
            directory: package_name.clone(),
            entries: selected,
        },
    );

    if let Err(error) = manifest.save(manifest_path) {
        rollback_launchers(&bin_dir, &created_launchers);
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }

    println!(
        "Installed package `{package_name}` with {} entries",
        names.len()
    );

    Ok(())
}

fn reject_symlinks(dir: &Path) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let metadata = fs::symlink_metadata(&path)?;
        let kind = metadata.file_type();

        if kind.is_symlink() {
            bail!("package contains a symbolic link: {}", path.display());
        }

        if kind.is_dir() {
            reject_symlinks(&path)?;
        } else if !kind.is_file() {
            bail!("unsupported filesystem entry: {}", path.display());
        }
    }

    Ok(())
}

#[cfg(unix)]
fn create_launcher(bin_dir: &Path, name: &str, target: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    let launcher = bin_dir.join(name);

    if util::path_occupied(&launcher) {
        bail!("launcher already exists: {}", launcher.display());
    }

    symlink(target, launcher)?;

    Ok(())
}

#[cfg(windows)]
fn create_launcher(bin_dir: &Path, name: &str, target: &Path) -> Result<()> {
    use std::io::Write;

    let launcher = bin_dir.join(format!("{name}.cmd"));

    let target = target.to_str()
        .context("launcher path is not valid Unicode")?;

    // In a batch file, %% represents a literal percent sign.
    let target = target.replace('%', "%%");

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&launcher)?;

    writeln!(file, "@echo off")?;
    writeln!(file, "rem binage-managed-launcher")?;
    writeln!(file, "setlocal DisableDelayedExpansion")?;
    writeln!(file, "\"{target}\" %*")?;
    writeln!(file, "exit /b %errorlevel%")?;

    file.sync_all()?;

    Ok(())
}

fn rollback_launchers(bin_dir: &Path, names: &[String]) {
    for name in names {
        #[cfg(unix)]
        let path = bin_dir.join(name);

        #[cfg(windows)]
        let path = bin_dir.join(format!("{name}.cmd"));

        let _ = fs::remove_file(path);
    }
}

