use anyhow::Result;

use crate::{
    config,
    entries,
    manifest::Manifest,
};

pub fn run() -> Result<()> {
    let root = config::root_from_manifest()?;
    let manifest = Manifest::load(&root.join(config::MANIFEST_FILE))?;

    let entries = entries::collect(&manifest);
    entries::print(&entries, None);

    Ok(())
}