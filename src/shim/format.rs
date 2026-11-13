use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use std::os::windows::ffi::{OsStrExt, OsStringExt};

const MAGIC: &[u8; 8] = b"BINAGSHM";
const VERSION: u32 = 1;
const FOOTER_SIZE: u64 = 16;

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// The embedded target must be a relative path inside packages/.
fn validate_target_path(path: &Path) -> io::Result<()> {
    let mut components = path.components();

    match components.next() {
        Some(Component::Normal(first)) if first == OsStr::new("packages") => {}
        _ => return Err(invalid_data("invalid shim target path")),
    }

    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(invalid_data("shim target path contains invalid components"));
    }

    Ok(())
}

/// Copy the shim template and append the target path metadata.
pub fn write_launcher(template: &[u8], output: &Path, target: &Path) -> io::Result<()> {
    validate_target_path(target)?;

    let wide: Vec<u16> = target.as_os_str().encode_wide().collect();

    if wide.is_empty() || wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid target path",
        ));
    }

    let path_len = u32::try_from(wide.len())
        .map_err(|_| io::Error::new(
            io::ErrorKind::InvalidInput,
            "target path is too long",
        ))?;

    let result = (|| -> io::Result<()> {
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;

        // Write the embedded PE template.
        output_file.write_all(template)?;

        // Append the UTF-16LE target path.
        for unit in &wide {
            output_file.write_all(&unit.to_le_bytes())?;
        }

        // Append metadata footer.
        output_file.write_all(&path_len.to_le_bytes())?;
        output_file.write_all(&VERSION.to_le_bytes())?;
        output_file.write_all(MAGIC)?;

        output_file.sync_all()?;

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(output);
    }

    result
}

/// Read and validate the embedded target path.
pub fn read_target(executable: &Path) -> io::Result<PathBuf> {
    let mut file = File::open(executable)?;
    let file_len = file.metadata()?.len();

    if file_len < FOOTER_SIZE {
        return Err(invalid_data("file is too small to be a binage shim"));
    }

    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;

    let mut footer = [0u8; 16];
    file.read_exact(&mut footer)?;

    let path_len = u32::from_le_bytes(footer[0..4].try_into().unwrap());
    let version = u32::from_le_bytes(footer[4..8].try_into().unwrap());

    if &footer[8..16] != MAGIC {
        return Err(invalid_data("invalid binage shim magic"));
    }

    if version != VERSION {
        return Err(invalid_data("unsupported binage shim version"));
    }

    if path_len == 0 {
        return Err(invalid_data("empty shim target path"));
    }

    let path_bytes = u64::from(path_len)
        .checked_mul(2)
        .ok_or_else(|| invalid_data("invalid shim target length"))?;

    let footer_start = file_len - FOOTER_SIZE;

    if path_bytes > footer_start {
        return Err(invalid_data("invalid shim target length"));
    }

    file.seek(SeekFrom::Start(footer_start - path_bytes))?;

    let mut bytes = vec![0u8; path_bytes as usize];
    file.read_exact(&mut bytes)?;

    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    if wide.contains(&0) {
        return Err(invalid_data("shim target contains a NUL character"));
    }

    let target = PathBuf::from(OsString::from_wide(&wide));

    validate_target_path(&target)?;

    Ok(target)
}
