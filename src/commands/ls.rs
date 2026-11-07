use crate::{config, manifest::Manifest};
use anyhow::Result;

pub fn run() -> Result<()> {
    let root = config::root_from_manifest()?;
    let manifest = Manifest::load(&root.join(config::MANIFEST_FILE))?;

    println!("{:<20} {:<10} LOCATION", "NAME", "TYPE");

    for (name, record) in &manifest.binaries {
        println!("{name:<20} {:<10} bin/{}", "binary", record.file_name);
    }

    for package in manifest.packages.values() {
        for (name, relative) in &package.entries {
            println!(
                "{name:<20} {:<10} packages/{}/{}",
                "package", package.directory, relative
            );
        }
    }

    Ok(())
}
