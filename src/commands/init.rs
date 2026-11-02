use anyhow::Result;
use std::path::PathBuf;
use crate::{config, manifest::Manifest, platform};

pub fn run(path: Option<PathBuf>) -> Result<()> {
    // The first successful initialization fixes the installation root.
    // Later `init` calls never relocate it, because the PATH entry is immutable.
    let root = match config::configured_root()? {
        Some(existing) => existing,
        None => config::prepare_root(path)?,
    };

    std::fs::create_dir_all(root.join("bin"))?;
    std::fs::create_dir_all(root.join("packages"))?;
    let manifest_path = root.join(config::MANIFEST_FILE);
    let manifest = Manifest::load(&manifest_path)?;
    manifest.save(&manifest_path)?;
    config::save_root_pointer(&root)?;
    let changed = platform::add_to_path(&root.join("bin"))?;
    println!("binage initialized at {}", root.display());
    if !changed {
        println!("PATH was already configured; it was left unchanged.");
    }
    Ok(())
}

