use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(default)]
    pub binaries: BTreeMap<String, BinaryRecord>,
    #[serde(default)]
    pub packages: BTreeMap<String, PackageRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BinaryRecord {
    pub file_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageRecord {
    pub directory: String,
    pub entries: BTreeMap<String, String>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                schema_version: 1,
                ..Default::default()
            });
        }
        let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        let mut value: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid manifest: {}", path.display()))?;
        if value.schema_version == 0 {
            value.schema_version = 1;
        }
        anyhow::ensure!(
            value.schema_version == 1,
            "unsupported manifest schema {}",
            value.schema_version
        );
        Ok(value)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes)?;
        replace_file(&temp, path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, destination: &Path) -> Result<()> {
    fs::rename(temp, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(temp: &Path, destination: &Path) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();

    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    let succeeded = unsafe { MoveFileExW(temp_wide.as_ptr(), destination_wide.as_ptr(), flags) };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}
