use anyhow::{bail, Result};

use crate::{
    config,
    entries,
    manifest::Manifest,
};

pub fn run(query: &str) -> Result<()> {
    if query.trim().is_empty() {
        bail!("Search query cannot be empty.");
    }

    let root = config::root_from_manifest()?;
    let manifest = Manifest::load(&root.join(config::MANIFEST_FILE))?;

    let all_entries = entries::collect(&manifest);
    let normalized_query = query.to_ascii_lowercase();

    let matched: Vec<_> = all_entries
        .into_iter()
        .filter(|entry| {
            entry.name.to_ascii_lowercase().contains(&normalized_query)
        })
        .collect();

    if matched.is_empty() {
        println!("No installed commands matching '{query}'.");
        return Ok(());
    }

    entries::print(&matched, Some(query));

    println!("\n{} result(s) found.", matched.len());

    Ok(())
}
