//! Where the app keeps its data.

use std::path::PathBuf;

/// `$ANYNOTE_DATA_DIR`, else `$XDG_DATA_HOME/anynote`, else
/// `~/.local/share/anynote`. The desktop app and the CLI share this default.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ANYNOTE_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("anynote");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/anynote")
}
