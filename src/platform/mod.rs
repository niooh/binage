#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub fn add_to_path(bin_dir: &std::path::Path) -> anyhow::Result<bool> {
    unix::add_to_path(bin_dir)
}

#[cfg(windows)]
pub fn add_to_path(bin_dir: &std::path::Path) -> anyhow::Result<bool> {
    windows::add_to_path(bin_dir)
}

#[cfg(unix)]
pub fn remove_from_path(bin_dir: &std::path::Path) -> anyhow::Result<bool> {
    unix::remove_from_path(bin_dir)
}

#[cfg(windows)]
pub fn remove_from_path(bin_dir: &std::path::Path) -> anyhow::Result<bool> {
    windows::remove_from_path(bin_dir)
}
