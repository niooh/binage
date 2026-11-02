use anyhow::{Context, Result, bail};
use std::{env, fs, path::{Path, PathBuf}};

pub const MANIFEST_FILE: &str = "manifest.json";

pub fn default_root() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = env::var_os("LOCALAPPDATA")
            .context("LOCALAPPDATA is not set")?;
        return Ok(PathBuf::from(base).join("binage"));
    }
    #[cfg(not(windows))]
    {
        let home = env::var_os("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home).join(".local").join("binage"));
    }
}

fn config_file() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = env::var_os("APPDATA")
            .or_else(|| env::var_os("LOCALAPPDATA"))
            .context("APPDATA and LOCALAPPDATA are not set")?;
        return Ok(PathBuf::from(base).join("binage").join("root.txt"));
    }
    #[cfg(not(windows))]
    {
        let home = env::var_os("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home).join(".config").join("binage").join("root.txt"));
    }
}

pub fn configured_root() -> Result<Option<PathBuf>> {
    let pointer = config_file()?;
    if !pointer.is_file() {
        return Ok(None);
    }
    let value = fs::read_to_string(&pointer)
        .with_context(|| format!("cannot read {}", pointer.display()))?;
    let value = value.trim();

    if value.is_empty() {
        bail!("The installation root pointer is empty.");
    }

    let root = PathBuf::from(value);
    if !root.is_absolute() {
        bail!("The installation root pointer must contain an absolute path.");
    }

    Ok(Some(root))
}

pub fn root_from_manifest() -> Result<PathBuf> {
    let root = match configured_root()? {
        Some(path) => path,
        None => default_root()?,
    };
    if root.join(MANIFEST_FILE).is_file() {
        return Ok(root);
    }
    bail!("binage is not initialized at {}. Run `binage init` first.", root.display())
}

pub fn save_root_pointer(root: &Path) -> Result<()> {
    let pointer = config_file()?;
    if let Some(parent) = pointer.parent() {
        fs::create_dir_all(parent)?;
    }
    if !pointer.exists() {
        fs::write(&pointer, root.to_string_lossy().as_bytes())
            .with_context(|| format!("cannot write {}", pointer.display()))?;
    }
    Ok(())
}

pub fn prepare_root(path: Option<PathBuf>) -> Result<PathBuf> {
    let root = path.unwrap_or(default_root()?);
    fs::create_dir_all(root.join("bin"))
        .with_context(|| format!("cannot create {}", root.display()))?;
    fs::create_dir_all(root.join("packages"))?;
    let root = fs::canonicalize(&root)
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    Ok(root)
}

pub fn ensure_inside(base: &Path, candidate: &Path) -> Result<()> {
    if !candidate.starts_with(base) {
        bail!("entry escapes the package directory: {}", candidate.display());
    }
    Ok(())
}

