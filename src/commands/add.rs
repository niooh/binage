use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    config::{self, MANIFEST_FILE},
    manifest::{Manifest, PackageRecord},
    util,
};

static BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
enum OverwritePolicy {
    Ask,
    Force,
}

pub fn run(source: PathBuf, entries: Vec<PathBuf>, force: bool) -> Result<()> {
    let policy = if force {
        OverwritePolicy::Force
    } else {
        OverwritePolicy::Ask
    };

    let root = config::root_from_manifest()?;
    let manifest_path = root.join(MANIFEST_FILE);

    let mut manifest = Manifest::load(&manifest_path)?;

    let source_meta = fs::symlink_metadata(&source)?;

    if source_meta.file_type().is_symlink() {
        bail!("source itself must not be a symbolic link");
    }

    let source = fs::canonicalize(&source)?;

    if source.is_file() {
        return add_binary(
            &root,
            &manifest_path,
            &mut manifest,
            &source,
            entries,
            policy,
        );
    }

    if source.is_dir() {
        return add_package(
            &root,
            &manifest_path,
            &mut manifest,
            &source,
            entries,
            policy,
        );
    }

    bail!("source must be a regular file or directory");
}

/// Only an explicit y/Y confirms overwriting.
fn confirm_overwrite(message: &str) -> Result<bool> {
    print!("{message} [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim(), "y" | "Y"))
}

fn authorize_overwrite(policy: OverwritePolicy, required: bool) -> Result<bool> {
    if !required || matches!(policy, OverwritePolicy::Force) {
        return Ok(true);
    }

    confirm_overwrite("Overwrite existing installation?")
}

fn validate_single_component(value: &str) -> Result<()> {
    let path = Path::new(value);

    if value.is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        bail!("Invalid managed path: {value}");
    }

    Ok(())
}

/// Create a temporary backup directory under the installation root.
/// Keeping it under root ensures that renames stay on the same filesystem.
fn create_backup_dir(root: &Path) -> Result<PathBuf> {
    loop {
        let id = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(".binage-backup-{}-{id}", std::process::id()));

        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

/// Move an existing path into the backup directory.
/// symlink_metadata is used so dangling symlinks are also detected.
fn move_to_backup(
    path: &Path,
    backup_dir: &Path,
    index: usize,
    backups: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }

    let backup = backup_dir.join(index.to_string());
    fs::rename(path, &backup)
        .with_context(|| format!("Cannot move {} into temporary backup", path.display()))?;

    backups.push((path.to_path_buf(), backup));
    Ok(())
}

/// Restore previously moved files/directories in reverse order.
fn restore_backups(backups: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut errors = Vec::new();

    for (original, backup) in backups.iter().rev() {
        if let Err(error) = fs::rename(backup, original) {
            errors.push(format!("{}: {}", original.display(), error));
        }
    }

    if !errors.is_empty() {
        bail!(
            "Failed to restore some previous files:\n{}",
            errors.join("\n")
        );
    }

    Ok(())
}

fn finish_backup(backup_dir: &Path) {
    let _ = fs::remove_dir_all(backup_dir);
}

/// Finish an installation transaction and restore staged files on failure.
///
/// The backup directory is deliberately retained if restoration fails, so
/// files that could not be restored are not deleted with the temporary backup.
fn finish_transaction<T>(
    backup_dir: &Path,
    backups: &[(PathBuf, PathBuf)],
    result: Result<T>,
    failure_context: &str,
    rollback: impl FnOnce(),
) -> Result<T> {
    match result {
        Ok(value) => {
            finish_backup(backup_dir);
            Ok(value)
        }
        Err(error) => {
            rollback();

            match restore_backups(backups) {
                Ok(()) => {
                    finish_backup(backup_dir);
                    Err(error)
                }
                Err(restore_error) => Err(restore_error.context(format!(
                    "{failure_context}: {error:#}. Backup files have been retained at {}",
                    backup_dir.display()
                ))),
            }
        }
    }
}

fn launcher_path(bin_dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        bin_dir.join(format!("{name}.exe"))
    }

    #[cfg(not(windows))]
    {
        bin_dir.join(name)
    }
}

fn validate_managed_binary(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if !metadata.file_type().is_file() {
        bail!(
            "Refusing to overwrite a path that is not a regular file: {}",
            path.display()
        );
    }

    Ok(())
}

#[cfg(unix)]
fn validate_managed_binary_alias(alias: &Path, binary: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(alias) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_symlink() {
        bail!(
            "Refusing to overwrite a command alias that is not a symbolic link: {}",
            alias.display()
        );
    }

    let actual = fs::canonicalize(alias)
        .with_context(|| format!("Cannot resolve command alias: {}", alias.display()))?;

    let expected = fs::canonicalize(binary)
        .with_context(|| format!("Cannot resolve installed binary: {}", binary.display()))?;

    if actual != expected {
        bail!(
            "Refusing to overwrite an alias pointing to a different file: {}",
            alias.display()
        );
    }

    Ok(())
}

fn add_binary(
    root: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
    source: &Path,
    entries: Vec<PathBuf>,
    policy: OverwritePolicy,
) -> Result<()> {
    if !entries.is_empty() {
        bail!("-e can only be used with a directory");
    }

    let name = util::command_name(source)?;
    let bin_dir = root.join("bin");

    let package_owner = manifest
        .packages
        .iter()
        .find_map(|(package_name, package)| {
            package
                .entries
                .contains_key(&name)
                .then(|| package_name.clone())
        });

    let package_owner_count = manifest
        .packages
        .values()
        .filter(|package| package.entries.contains_key(&name))
        .count();

    if package_owner_count > 1 {
        bail!("command `{name}` is registered by multiple packages");
    }

    if package_owner.is_some() && manifest.binaries.contains_key(&name) {
        bail!("command `{name}` has conflicting binary and package records");
    }

    let old_record = manifest.binaries.get(&name);
    let has_old_record = old_record.is_some();
    let file_name = source.file_name().context("missing filename")?;
    let destination = bin_dir.join(file_name);

	let old_binary = if let Some(file_name) = old_record {
		validate_single_component(file_name)?;

		let old_path = bin_dir.join(file_name);
        if util::path_occupied(&old_path) {
            validate_managed_binary(&old_path)?;
        }

        #[cfg(unix)]
        {
            let alias = bin_dir.join(&name);
            if alias != old_path && util::path_occupied(&alias) {
                validate_managed_binary_alias(&alias, &old_path)?;
            }
        }

        Some(old_path)
    } else {
        None
    };

	let package_launcher = if let Some(package_name) = &package_owner {
		let package = manifest
			.packages
			.get(package_name)
			.context("The package record disappeared unexpectedly")?;

		validate_single_component(package_name)?;

		let launcher = launcher_path(&bin_dir, &name);
		if util::path_occupied(&launcher) {
			validate_managed_launcher(&launcher, &bin_dir, &root.join("packages"))?;
		}

		Some((
			launcher,
			root.join("packages").join(package_name),
			package.entries.len() == 1,
		))
	} else {
		None
	};

    if util::path_occupied(&destination) {
        let is_old_binary = old_binary.as_deref() == Some(destination.as_path());
        let is_package_launcher = package_launcher
            .as_ref()
            .map_or(false, |(launcher, _, _)| launcher == &destination);

        if !is_old_binary && !is_package_launcher {
            bail!(
                "destination already exists and is not managed by binage: {}",
                destination.display()
            );
        }
    }

    if !authorize_overwrite(policy, has_old_record || package_launcher.is_some())? {
        println!("Cancelled.");
        return Ok(());
    }

    let backup_dir = create_backup_dir(root)?;
    let mut backups = Vec::new();

    let result = (|| -> Result<()> {
        // Stage the old binary.
        if let Some(old_path) = &old_binary {
            move_to_backup(old_path, &backup_dir, backups.len(), &mut backups)?;
        }

        // Stage the old package launcher and remove its package directory
        // only when this was the package's final command.
        if let Some((launcher, package_dir, is_last_entry)) = &package_launcher {
            move_to_backup(launcher, &backup_dir, backups.len(), &mut backups)?;
            if *is_last_entry {
                move_to_backup(package_dir, &backup_dir, backups.len(), &mut backups)?;
            }
        }

        // Stage the Unix extensionless .exe alias, if present.
        #[cfg(unix)]
        if has_old_record {
            let alias = bin_dir.join(&name);
            if Some(alias.as_path()) != old_binary.as_deref() {
                move_to_backup(&alias, &backup_dir, backups.len(), &mut backups)?;
            }
        }

        // Check for other command-name conflicts after staging the
        // old managed binary and its alias.
        util::check_entry_available(&bin_dir, &name)?;

        if util::path_occupied(&destination) {
            bail!("destination already exists: {}", destination.display());
        }

        util::copy_file(source, &destination)?;
        util::ensure_executable(&destination)?;

        // Unix needs an extensionless alias for .exe files.
        #[cfg(unix)]
        if name != file_name.to_string_lossy() {
            use std::os::unix::fs::symlink;

            let alias = bin_dir.join(&name);
            symlink(&destination, &alias)?;
        }

        if let Some(package_name) = &package_owner {
            let package_is_empty = {
                let package = manifest
                    .packages
                    .get_mut(package_name)
                    .context("The package record disappeared unexpectedly")?;

                package.entries.remove(&name);
                package.entries.is_empty()
            };

            if package_is_empty {
                manifest.packages.remove(package_name);
            }
        }

        manifest.binaries.insert(
            name.clone(),
            file_name.to_string_lossy().into_owned(),
        );

        manifest.save(manifest_path)?;

        Ok(())
    })();

    finish_transaction(
        &backup_dir,
        &backups,
        result,
        "Installation failed",
        || {
            let _ = fs::remove_file(&destination);

            #[cfg(unix)]
            let _ = fs::remove_file(bin_dir.join(&name));
        },
    )?;

    println!("Installed binary: {name}");

    Ok(())
}

fn add_package(
    root: &Path,
    manifest_path: &Path,
    manifest: &mut Manifest,
    source: &Path,
    entries: Vec<PathBuf>,
    policy: OverwritePolicy,
) -> Result<()> {
    if entries.is_empty() {
        bail!("directory installs require at least one -e FILE");
    }

    let package_name = source
        .file_name()
        .context("directory has no filename")?
        .to_string_lossy()
        .into_owned();

    let packages_dir = root.join("packages");
    let bin_dir = root.join("bin");

    let old_package = manifest.packages.get(&package_name);
	validate_single_component(&package_name)?;
	let destination = packages_dir.join(&package_name);

    if old_package.is_none() && util::path_occupied(&destination) {
        bail!(
            "package directory already exists and is not managed by binage: {}",
            destination.display()
        );
    }

    if old_package.is_some() && util::path_occupied(&destination) {
        let metadata = fs::symlink_metadata(&destination)?;

        if !metadata.file_type().is_dir() {
            bail!(
                "Refusing to overwrite a package path that is not a directory: {}",
                destination.display()
            );
        }
    }

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

        // A standalone binary with this name can be replaced transactionally.
        let belongs_to_other_package = manifest.packages.iter().any(|(existing_name, package)| {
            existing_name != &package_name && package.entries.contains_key(&name)
        });

        if belongs_to_other_package {
            bail!("entry name `{name}` belongs to another installed package");
        }

        if selected.contains_key(&name) {
            bail!("duplicate entry name `{name}`");
        }

        let relative = candidate.strip_prefix(source)?.to_path_buf();

        selected.insert(name.clone(), relative.to_string_lossy().into_owned());

        names.push((name, relative));
    }


    // Collect binaries whose command names are being transferred to this package.
    let mut binary_replacements = Vec::new();

    for (name, _) in &names {
        if let Some(file_name) = manifest.binaries.get(name) {
			validate_single_component(file_name)?;

			let binary_path = bin_dir.join(file_name);
            if util::path_occupied(&binary_path) {
                validate_managed_binary(&binary_path)?;
            }

            #[cfg(unix)]
            let binary_alias = {
                let alias = bin_dir.join(name);
                if alias != binary_path && util::path_occupied(&alias) {
                    validate_managed_binary_alias(&alias, &binary_path)?;
                    Some(alias)
                } else {
                    None
                }
            };

            #[cfg(not(unix))]
            let binary_alias: Option<PathBuf> = None;

            binary_replacements.push((name.clone(), binary_path, binary_alias));
        }
    }

    // Validate all existing launchers belonging to the package before
    // moving or replacing anything.
    let mut old_launchers = Vec::new();

    if let Some(package) = old_package {
        for name in package.entries.keys() {
            let launcher = launcher_path(&bin_dir, name);

            if util::path_occupied(&launcher) {
                validate_managed_launcher(&launcher, &bin_dir, &packages_dir)?;

                old_launchers.push(launcher);
            }
        }
    }

    if !authorize_overwrite(
        policy,
        old_package.is_some() || !binary_replacements.is_empty(),
    )? {
        println!("Cancelled.");
        return Ok(());
    }

    let backup_dir = create_backup_dir(root)?;
    let mut backups = Vec::new();
    let mut created_launchers = Vec::new();

    let result = (|| -> Result<()> {
        // Stage old package launchers first.
        let mut staged_paths = Vec::new();
        for launcher in &old_launchers {
            if !staged_paths.contains(launcher) {
                move_to_backup(launcher, &backup_dir, backups.len(), &mut backups)?;
                staged_paths.push(launcher.clone());
            }
        }

        // Stage the old standalone binaries and their Unix aliases.
        for (_, binary_path, alias) in &binary_replacements {
            if !staged_paths.contains(binary_path) {
                move_to_backup(binary_path, &backup_dir, backups.len(), &mut backups)?;
                staged_paths.push(binary_path.clone());
            }

            if let Some(alias) = alias {
                if !staged_paths.contains(alias) {
                    move_to_backup(alias, &backup_dir, backups.len(), &mut backups)?;
                    staged_paths.push(alias.clone());
                }
            }
        }

        // Stage the old package directory.
        move_to_backup(&destination, &backup_dir, backups.len(), &mut backups)?;

        // Check that new names do not collide with unrelated files.
        for (name, _) in &names {
            util::check_entry_available(&bin_dir, name)?;
        }

        // Copy the complete package directory.
        util::copy_dir(source, &destination)?;

        for (name, relative) in &names {
            let target = destination.join(relative);

            util::ensure_executable(&target)?;
            create_launcher(&bin_dir, name, &target)?;
            created_launchers.push(name.clone());
        }

        // Transfer command ownership from standalone binaries to this package.
        for (name, _, _) in &binary_replacements {
            manifest.binaries.remove(name);
        }

		manifest.packages.insert(
			package_name.clone(),
			PackageRecord {
				entries: selected,
			},
		);

        manifest.save(manifest_path)?;

        Ok(())
    })();

    finish_transaction(
        &backup_dir,
        &backups,
        result,
        "Package installation failed",
        || {
            rollback_launchers(&bin_dir, &created_launchers);
            let _ = fs::remove_dir_all(&destination);
        },
    )?;

    println!(
        "Installed package `{package_name}` with {} entries",
        names.len()
    );

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
fn create_launcher(
    bin_dir: &Path,
    name: &str,
    target: &Path,
) -> Result<()> {
    let launcher = bin_dir.join(format!("{name}.exe"));

    if util::path_occupied(&launcher) {
        bail!("launcher already exists: {}", launcher.display());
    }

    let root = bin_dir
        .parent()
        .context("bin directory has no parent")?;

    let relative_target = target
        .strip_prefix(root)
        .context("launcher target is outside binage root")?;

	crate::shim::write_launcher(
		crate::SHIM_TEMPLATE,
		&launcher,
		relative_target,
	)
    .with_context(|| {
        format!(
            "failed to create launcher: {}",
            launcher.display()
        )
    })?;

    Ok(())
}

fn rollback_launchers(bin_dir: &Path, names: &[String]) {
    for name in names {
        let path = launcher_path(bin_dir, name);
        let _ = fs::remove_file(path);
    }
}

/// Verify that an existing launcher belongs to binage.
fn validate_managed_launcher(launcher: &Path, bin_dir: &Path, packages_dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(launcher)?;

    #[cfg(unix)]
    {
        if !metadata.file_type().is_symlink() {
            bail!(
                "Refusing to overwrite a launcher that is not a symbolic link: {}",
                launcher.display()
            );
        }

        let target = fs::canonicalize(launcher)
            .with_context(|| format!("Cannot resolve launcher target: {}", launcher.display()))?;

        let packages_root = fs::canonicalize(packages_dir).with_context(|| {
            format!(
                "Cannot resolve package directory: {}",
                packages_dir.display()
            )
        })?;

        if !target.starts_with(&packages_root) {
            bail!(
                "Refusing to overwrite a launcher pointing outside the package directory: {}",
                launcher.display()
            );
        }

        let _ = bin_dir;
    }

    #[cfg(windows)]
    {
        if !metadata.file_type().is_file() {
            bail!(
                "Refusing to overwrite a launcher that is not a regular file: {}",
                launcher.display()
            );
        }

        let relative_target = crate::shim::read_target(launcher)
            .with_context(|| format!("Cannot validate binage launcher: {}", launcher.display()))?;

        let root = bin_dir
            .parent()
            .context("bin directory has no parent")?;
        let resolved_target = fs::canonicalize(root.join(relative_target))
            .with_context(|| format!("Cannot resolve launcher target: {}", launcher.display()))?;
        let packages_root = fs::canonicalize(packages_dir).with_context(|| {
            format!(
                "Cannot resolve package directory: {}",
                packages_dir.display()
            )
        })?;

        if !resolved_target.starts_with(&packages_root) {
            bail!(
                "Refusing to overwrite a launcher pointing outside the package directory: {}",
                launcher.display()
            );
        }
    }

    Ok(())
}
