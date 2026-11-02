use anyhow::{Context, Result};
use std::{env, fs, path::Path};

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
    let home = std::path::PathBuf::from(home);

    let config = if shell.ends_with("/fish") {
        let dir = home.join(".config/fish/conf.d");
        fs::create_dir_all(&dir)?;
        dir.join("binage.fish")
    } else if shell.ends_with("/zsh") {
        home.join(".zshrc")
    } else if shell.ends_with("/bash") {
        let bashrc = home.join(".bashrc");
        if bashrc.exists() { bashrc } else { home.join(".profile") }
    } else {
        home.join(".profile")
    };

    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = fs::read_to_string(&config).unwrap_or_default();
    if existing.contains(BEGIN) {
        return Ok(false);
    }

    let path = bin_dir.to_string_lossy();
    let block = if shell.ends_with("/fish") {
        format!("{BEGIN}\nfish_add_path --global {}\n{END}\n", fish_quote(&path))
    } else {
        format!("{BEGIN}\nexport PATH={}:\"$PATH\"\n{END}\n", shell_quote(&path))
    };

    use std::io::Write;
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&config)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.write_all(block.as_bytes())?;
    println!("Added {} to PATH in {}", bin_dir.display(), config.display());
    Ok(true)
}

