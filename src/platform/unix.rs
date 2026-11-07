use anyhow::{Context, Result};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

const BEGIN: &str = "# >>> binage PATH >>>";
const END: &str = "# <<< binage PATH <<<";

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

pub fn add_to_path(bin_dir: &Path) -> Result<bool> {
    let home = env::var_os("HOME").context("HOME is not set")?;

    let shell = env::var("SHELL").unwrap_or_default();
    let home = PathBuf::from(home);

    let config = if shell.ends_with("/fish") {
        let dir = home.join(".config/fish/conf.d");
        fs::create_dir_all(&dir)?;
        dir.join("binage.fish")
    } else if shell.ends_with("/zsh") {
        home.join(".zshrc")
    } else if shell.ends_with("/bash") {
        let bashrc = home.join(".bashrc");

        if bashrc.exists() {
            bashrc
        } else {
            home.join(".profile")
        }
    } else {
        home.join(".profile")
    };

    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = match fs::read_to_string(&config) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot read {}", config.display()));
        }
    };

    if existing.contains(BEGIN) {
        return Ok(false);
    }

    let path = bin_dir.to_string_lossy();

    let block = if shell.ends_with("/fish") {
        format!(
            "{BEGIN}\nfish_add_path --global {}\n{END}\n",
            fish_quote(&path)
        )
    } else {
        format!(
            "{BEGIN}\nexport PATH={}:\"$PATH\"\n{END}\n",
            shell_quote(&path)
        )
    };

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config)?;

    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }

    file.write_all(block.as_bytes())?;

    println!(
        "Added {} to PATH in {}",
        bin_dir.display(),
        config.display()
    );

    Ok(true)
}

/// Remove binage's managed block from file contents.
///
/// The original line endings and final-newline state are preserved.
fn remove_path_block(contents: &str, path: &Path) -> Result<Option<String>> {
    let mut updated = String::with_capacity(contents.len());
    let mut inside_block = false;
    let mut removed = false;

    for segment in contents.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let marker = line.trim();

        if marker == BEGIN {
            if inside_block {
                anyhow::bail!("Nested binage PATH block found in {}", path.display());
            }

            inside_block = true;
            removed = true;
            continue;
        }

        if marker == END {
            if !inside_block {
                anyhow::bail!(
                    "Unmatched binage PATH block terminator in {}",
                    path.display()
                );
            }

            inside_block = false;
            continue;
        }

        if !inside_block {
            updated.push_str(segment);
        }
    }

    if inside_block {
        anyhow::bail!("Unterminated binage PATH block found in {}", path.display());
    }

    if removed {
        Ok(Some(updated))
    } else {
        Ok(None)
    }
}

/// Remove binage's PATH configuration from the user's shell startup files.
pub fn remove_from_path(_bin_dir: &Path) -> Result<bool> {
    let home = env::var_os("HOME").context("HOME is not set")?;

    let home = PathBuf::from(home);

    let files = [
        (home.join(".config/fish/conf.d/binage.fish"), true),
        (home.join(".zshrc"), false),
        (home.join(".bashrc"), false),
        (home.join(".profile"), false),
    ];

    // Validate all files before changing any of them.
    let mut changes = Vec::new();

    for (path, is_fish_file) in files {
        if !path.is_file() {
            continue;
        }

        let contents =
            fs::read_to_string(&path).with_context(|| format!("Cannot read {}", path.display()))?;

        if let Some(updated) = remove_path_block(&contents, &path)? {
            changes.push((path, is_fish_file, updated));
        }
    }

    let changed = !changes.is_empty();

    for (path, is_fish_file, updated) in changes {
        if is_fish_file && updated.trim().is_empty() {
            fs::remove_file(&path).with_context(|| format!("Cannot remove {}", path.display()))?;
        } else {
            fs::write(&path, updated)
                .with_context(|| format!("Cannot update {}", path.display()))?;
        }
    }

    Ok(changed)
}
