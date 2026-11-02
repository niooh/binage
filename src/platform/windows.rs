use anyhow::{bail, Context, Result};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::{
    Foundation::ERROR_FILE_NOT_FOUND,
    System::Registry::{
        RegCloseKey,
        RegOpenKeyExW,
        RegQueryValueExW,
        RegSetValueExW,
        HKEY,
        HKEY_CURRENT_USER,
        KEY_QUERY_VALUE,
        KEY_SET_VALUE,
        REG_EXPAND_SZ,
        REG_SZ,
    },
};

const ERROR_SUCCESS: u32 = 0;

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read the current user's PATH registry value.
///
/// `Ok(None)` means the PATH value does not exist.
/// All other registry errors are returned as errors.
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
        bail!("Failed to query the PATH registry value (error code: {status}).");
    }

    if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
        bail!("The PATH registry value has an unsupported registry type.");
    }

    if byte_len == 0 || byte_len % 2 != 0 {
        bail!("The PATH registry value has an invalid UTF-16 data length.");
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
        bail!("Failed to read the PATH registry value (error code: {status}).");
    }

    if actual_type != value_type {
        bail!("The PATH registry value changed while it was being read.");
    }

    if actual_len == 0 || actual_len % 2 != 0 || actual_len > byte_len {
        bail!("The PATH registry value returned an invalid data length.");
    }

    buffer.truncate((actual_len / 2) as usize);

    // Remove registry string terminators before appending anything.
    while buffer.last() == Some(&0) {
        buffer.pop();
    }

    // Embedded NUL characters could cause silent string truncation.
    if buffer.contains(&0) {
        bail!("The PATH registry value contains an embedded NUL character.");
    }

    let value = String::from_utf16(&buffer)
        .context("The PATH registry value is not valid UTF-16.")?;

    Ok(Some((value_type, value)))
}

fn normalize_path_entry(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn path_contains(path_value: &str, target: &str) -> bool {
    let target = normalize_path_entry(target);

    path_value
        .split(';')
        .any(|entry| normalize_path_entry(entry) == target)
}

/// Add the binage bin directory to the current user's PATH.
///
/// The registry is never written if reading PATH fails.
/// Existing PATH entries are preserved.
pub fn add_to_path(bin_dir: &Path) -> Result<bool> {
    let target = bin_dir
        .to_str()
        .context("The bin directory path is not valid Unicode.")?
        .to_owned();

    if target.contains(';') || target.contains('\0') {
        bail!("The bin directory path contains an unsupported character.");
    }

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

    let result = (|| -> Result<bool> {
        // A failed read aborts the operation. It is never treated as an empty PATH.
        let original = unsafe { read_path_value(key)? };

        let (original_type, original_value) = match &original {
            Some((kind, value)) => (*kind, value.clone()),
            None => (REG_SZ, String::new()),
        };

        if path_contains(&original_value, &target) {
            return Ok(false);
        }

        let mut updated = original_value.to_owned();

        if !updated.is_empty() && !updated.ends_with(';') {
            updated.push(';');
        }

        updated.push_str(&target);

        // Re-read PATH immediately before writing to reduce the chance of
        // overwriting a concurrent change made by another process.
        let current = unsafe { read_path_value(key)? };

        let unchanged = match (&original, &current) {
            (None, None) => true,
            (Some((old_type, old_value)), Some((new_type, new_value))) => {
                old_type == new_type && old_value == new_value
            }
            _ => false,
        };

        if !unchanged {
            bail!("PATH changed during the operation. No registry update was performed.");
        }

        let mut data: Vec<u16> = updated.encode_utf16().collect();
        data.push(0);

        let byte_len = data
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .and_then(|length| u32::try_from(length).ok())
            .context("The PATH value is too large for the Windows registry API.")?;

        let value_name = wide_null("PATH");

        let status = unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                original_type,
                data.as_ptr().cast::<u8>(),
                byte_len,
            )
        };

        if status != ERROR_SUCCESS {
            bail!("Failed to update the PATH registry value (error code: {status}).");
        }

        Ok(true)
    })();

    unsafe {
        RegCloseKey(key);
    }

    result
}

