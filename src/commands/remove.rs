use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{
    config::{self, root_from_manifest},
    manifest::Manifest,
};

/// Remove an installed command or package entry.
pub fn run(name: &str) -> Result<()> {
    if name == "binage" {
        bail!("cannot remove the binage executable itself");
    }
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        bail!("Invalid command name: {name}");
    }

    let root = root_from_manifest()?;
    let bin_dir = root.join("bin");
    let packages_dir = root.join("packages");
    let manifest_path = root.join(config::MANIFEST_FILE);

    let mut manifest = Manifest::load(&manifest_path)?;

	if let Some(file_name) = manifest.binaries.get(name) {
		validate_single_component(file_name)?;
		let binary_path = bin_dir.join(file_name);

        // On Unix, an .exe binary also has an extensionless symlink.
        #[cfg(unix)]
        let alias_path = bin_dir.join(name);

        #[cfg(unix)]
        let remove_alias = is_managed_binary_alias(&alias_path, &binary_path)?;

        manifest.binaries.remove(name);
        manifest.save(&manifest_path)?;

        remove_regular_file_if_present(&binary_path)?;

        #[cfg(unix)]
        if remove_alias {
            fs::remove_file(&alias_path).with_context(|| {
                format!("Failed to remove command alias: {}", alias_path.display())
            })?;
        }

        println!("Removed command '{name}'.");
        return Ok(());
    }

    let package_name = manifest
        .packages
        .iter()
        .find_map(|(package_name, package)| {
            package
                .entries
                .contains_key(name)
                .then(|| package_name.clone())
        });

    let Some(package_name) = package_name else {
        bail!("No installed command or package entry named '{name}' was found.");
    };

    let package = manifest
        .packages
        .get(&package_name)
        .context("The package record disappeared unexpectedly.")?;

    let launcher_path = launcher_path(&bin_dir, name);

    // Validate ownership before changing the manifest.
    let launcher_exists = validate_managed_launcher(&launcher_path, &bin_dir, &packages_dir)?;

    validate_single_component(&package.directory)?;
    let package_directory = packages_dir.join(&package.directory);

    let package_is_empty = package.entries.len() == 1;

    if package_is_empty {
        manifest.packages.remove(&package_name);
    } else if let Some(package_record) = manifest.packages.get_mut(&package_name) {
        package_record.entries.remove(name);
    }

    // Persist the manifest first. If filesystem cleanup later fails,
    // the command is no longer registered and any leftover file is an orphan.
    manifest.save(&manifest_path)?;

    if launcher_exists {
        fs::remove_file(&launcher_path)
            .with_context(|| format!("Failed to remove launcher: {}", launcher_path.display()))?;
    }

    if package_is_empty {
        remove_package_directory_if_present(&package_directory)?;
    }

    println!("Removed command '{name}' from package '{package_name}'.");
    Ok(())
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

/// Validate that an existing launcher belongs to binage.
///
/// Returns false if the launcher does not exist.
fn validate_managed_launcher(launcher: &Path, bin_dir: &Path, packages_dir: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(launcher) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect launcher: {}", launcher.display()));
        }
    };

    #[cfg(unix)]
    {
        if !metadata.file_type().is_symlink() {
            bail!(
                "Refusing to remove a launcher that is not a symbolic link: {}",
                launcher.display()
            );
        }

        let link_target = fs::read_link(launcher)
            .with_context(|| format!("Failed to read launcher symlink: {}", launcher.display()))?;

        let resolved_target = if link_target.is_absolute() {
            lexical_normalize(&link_target)?
        } else {
            lexical_normalize(&bin_dir.join(link_target))?
        };

        let packages_root = fs::canonicalize(packages_dir).with_context(|| {
            format!(
                "Failed to resolve package directory: {}",
                packages_dir.display()
            )
        })?;

        if !resolved_target.starts_with(&packages_root) {
            bail!(
                "Refusing to remove a launcher that points outside the package directory: {}",
                launcher.display()
            );
        }

        Ok(true)
    }

    #[cfg(windows)]
    {
        if !metadata.file_type().is_file() {
            bail!(
                "Refusing to remove a launcher that is not a regular file: {}",
                launcher.display()
            );
        }

        // Read the embedded target path.
        let relative_target = crate::shim::read_target(launcher)
            .with_context(|| format!("Failed to validate binage shim: {}", launcher.display()))?;

        let root = bin_dir.parent().context("bin directory has no parent")?;

        let resolved_target = root.join(relative_target);

        if !resolved_target.starts_with(packages_dir) {
            bail!(
                "Refusing to remove a shim that points outside packages: {}",
                launcher.display()
            );
        }

        Ok(true)
    }
}

/// Normalize a path lexically without requiring the final target to exist.
#[cfg(unix)]
fn lexical_normalize(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    bail!("Cannot normalize a path that escapes its root.");
                }
            }
            Component::Normal(part) => result.push(part),
        }
    }

    Ok(result)
}

/// Check whether the Unix extensionless alias points to the installed binary.
#[cfg(unix)]
fn is_managed_binary_alias(alias: &Path, binary: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(alias) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_symlink() {
        bail!(
            "Refusing to remove a command alias that is not a symbolic link: {}",
            alias.display()
        );
    }

    let target = fs::read_link(alias)
        .with_context(|| format!("Failed to read command alias: {}", alias.display()))?;

    let expected = fs::canonicalize(binary)
        .with_context(|| format!("Failed to resolve installed binary: {}", binary.display()))?;

    let actual = if target.is_absolute() {
        fs::canonicalize(&target)
    } else {
        fs::canonicalize(
            alias
                .parent()
                .context("The command alias has no parent directory.")?
                .join(target),
        )
    }
    .with_context(|| {
        format!(
            "Failed to resolve command alias target: {}",
            alias.display()
        )
    })?;

    if actual != expected {
        bail!(
            "Refusing to remove a command alias that points to a different file: {}",
            alias.display()
        );
    }

    Ok(true)
}

fn remove_regular_file_if_present(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_file() {
        bail!(
            "Refusing to remove an installed binary that is not a regular file: {}",
            path.display()
        );
    }

    fs::remove_file(path)
        .with_context(|| format!("Failed to remove installed binary: {}", path.display()))
}

fn remove_package_directory_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                bail!(
                    "Refusing to remove a package path that is not a directory: {}",
                    path.display()
                );
            }

            fs::remove_dir_all(path)
                .with_context(|| format!("Failed to remove package directory: {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
