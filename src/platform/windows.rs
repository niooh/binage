use anyhow::{bail, Context, Result};
use std::{path::Path, ptr};

use windows_sys::Win32::{
    Foundation::ERROR_FILE_NOT_FOUND,
    System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
    },
};

const ERROR_SUCCESS: u32 = 0;

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Convert a canonical Windows path into a normal path suitable for PATH.
///
/// This conversion applies only to the path being added or removed.
/// Existing PATH entries are never rewritten or migrated.
fn path_for_path_variable(path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("Cannot resolve {}", path.display()))?;

    let value = canonical
        .to_str()
        .context("The bin directory path is not valid Unicode.")?;

    let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        value.to_owned()
    };

    if value.contains(';') || value.contains('\0') {
        bail!("The bin directory path contains an unsupported character.");
    }

    Ok(value)
}

/// Normalize ordinary PATH entries for comparison.
///
/// Deliberately does not remove the `\\?\` prefix.
/// Therefore, legacy verbatim paths are not considered equivalent.
fn normalize_path_entry(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Read the current user's PATH registry value.
///
/// `Ok(None)` means the PATH value does not exist.
unsafe fn read_path_value(key: HKEY) -> Result<Option<(u32, String)>> {
    let value_name = wide_null("PATH");

    let mut value_type = 0u32;
    let mut byte_len = 0u32;

    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            &mut value_type,
            ptr::null_mut(),
            &mut byte_len,
        )
    };

    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }

    if status != ERROR_SUCCESS {
        bail!("Failed to query PATH (error code: {status}).");
    }

    if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
        bail!("The PATH registry value has an unsupported type.");
    }

    if byte_len == 0 || byte_len % 2 != 0 {
        bail!("The PATH registry value has an invalid UTF-16 length.");
    }

    let mut buffer = vec![0u16; (byte_len / 2) as usize];
    let mut actual_len = byte_len;
    let mut actual_type = 0u32;

    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            &mut actual_type,
            buffer.as_mut_ptr().cast::<u8>(),
            &mut actual_len,
        )
    };

    if status != ERROR_SUCCESS {
        bail!("Failed to read PATH (error code: {status}).");
    }

    if actual_type != value_type {
        bail!("The PATH registry value changed while being read.");
    }

    if actual_len == 0 || actual_len % 2 != 0 || actual_len > byte_len {
        bail!("The PATH registry returned an invalid data length.");
    }

    buffer.truncate((actual_len / 2) as usize);

    while buffer.last() == Some(&0) {
        buffer.pop();
    }

    if buffer.contains(&0) {
        bail!("The PATH registry value contains an embedded NUL.");
    }

    let value =
        String::from_utf16(&buffer).context("The PATH registry value is not valid UTF-16.")?;

    Ok(Some((value_type, value)))
}

/// Write the user's PATH registry value.
fn write_path_value(key: HKEY, value_type: u32, value: &str) -> Result<()> {
    let value_name = wide_null("PATH");

    let mut data: Vec<u16> = value.encode_utf16().collect();
    data.push(0);

    let byte_len = data
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|len| u32::try_from(len).ok())
        .context("The PATH value is too large.")?;

    let status = unsafe {
        RegSetValueExW(
            key,
            value_name.as_ptr(),
            0,
            value_type,
            data.as_ptr().cast::<u8>(),
            byte_len,
        )
    };

    if status != ERROR_SUCCESS {
        bail!("Failed to update PATH (error code: {status}).");
    }

    Ok(())
}

/// Open the current user's environment registry key.
fn open_environment_key() -> Result<HKEY> {
    let subkey = wide_null("Environment");
    let mut key: HKEY = ptr::null_mut();

    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };

    if status != ERROR_SUCCESS {
        bail!("Failed to open the user environment registry key (error code: {status}).");
    }

    Ok(key)
}

/// Check whether PATH changed since it was read.
fn path_unchanged(original: &Option<(u32, String)>, current: &Option<(u32, String)>) -> bool {
    match (original, current) {
        (None, None) => true,
        (Some((old_type, old_value)), Some((new_type, new_value))) => {
            old_type == new_type && old_value == new_value
        }
        _ => false,
    }
}

/// Add binage's bin directory to the current user's PATH.
///
/// Existing entries are preserved. No legacy path migration is performed.
pub fn add_to_path(bin_dir: &Path) -> Result<bool> {
    let target = path_for_path_variable(bin_dir)?;
    let target_normalized = normalize_path_entry(&target);

    let key = open_environment_key()?;

    let result = (|| -> Result<bool> {
        let original = unsafe { read_path_value(key)? };

        let (value_type, original_value) = match &original {
            Some((kind, value)) => (*kind, value.clone()),
            None => (REG_SZ, String::new()),
        };

        let already_exists = original_value
            .split(';')
            .any(|entry| normalize_path_entry(entry) == target_normalized);

        if already_exists {
            return Ok(false);
        }

        let updated = if original_value.is_empty() {
            target.clone()
        } else if original_value.ends_with(';') {
            format!("{original_value}{target}")
        } else {
            format!("{original_value};{target}")
        };

        // Avoid overwriting a concurrent PATH modification.
        let current = unsafe { read_path_value(key)? };

        if !path_unchanged(&original, &current) {
            bail!("PATH changed during the operation. No update was performed.");
        }

        write_path_value(key, value_type, &updated)?;

        Ok(true)
    })();

    unsafe {
        RegCloseKey(key);
    }

    result
}

/// Remove binage's bin directory from the current user's PATH.
///
/// Only the exact ordinary path is removed.
/// Other entries, including legacy verbatim paths, are left untouched.
pub fn remove_from_path(bin_dir: &Path) -> Result<bool> {
    let target = path_for_path_variable(bin_dir)?;
    let target_normalized = normalize_path_entry(&target);

    let key = open_environment_key()?;

    let result = (|| -> Result<bool> {
        let original = unsafe { read_path_value(key)? };

        let Some((value_type, original_value)) = original.clone() else {
            return Ok(false);
        };

        let entries: Vec<&str> = original_value.split(';').collect();

        let remaining: Vec<&str> = entries
            .iter()
            .copied()
            .filter(|entry| normalize_path_entry(entry) != target_normalized)
            .collect();

        if remaining.len() == entries.len() {
            return Ok(false);
        }

        let updated = remaining.join(";");

        // Avoid overwriting a concurrent PATH modification.
        let current = unsafe { read_path_value(key)? };

        if !path_unchanged(&original, &current) {
            bail!("PATH changed during the operation. No update was performed.");
        }

        write_path_value(key, value_type, &updated)?;

        Ok(true)
    })();

    unsafe {
        RegCloseKey(key);
    }

    result
}
