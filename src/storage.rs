//! Local JSON persistence. No server, no account — everything lives in a
//! single `store.json` under the OS-appropriate per-user data directory.

use crate::model::Store;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolve (and create) the path to the data file.
///
/// Windows: `%APPDATA%\VORA\Margin\data\store.json`
/// macOS:   `~/Library/Application Support/com.VORA.Margin/store.json`
/// Linux:   `~/.local/share/margin/store.json`
pub fn data_file() -> Result<PathBuf> {
    let pd = ProjectDirs::from("com", "VORA", "Margin")
        .context("could not determine a data directory for this OS")?;
    let dir = pd.data_dir();
    fs::create_dir_all(dir).with_context(|| format!("creating data dir {}", dir.display()))?;
    Ok(dir.join("store.json"))
}

/// The result of loading the store on startup.
pub struct Loaded {
    pub store: Store,
    /// Set when an unreadable `store.json` was moved aside so the app could
    /// still start. Holds the path the corrupt file was renamed to.
    pub recovered_backup: Option<PathBuf>,
}

/// Where a corrupt store file gets moved, tagged with a timestamp so repeated
/// recoveries never clobber each other.
fn backup_path(path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    path.with_file_name(format!("store.corrupt-{ts}.json"))
}

/// Load the store, or return an empty one on first run.
///
/// If `store.json` exists but can't be parsed (corruption, a truncated write,
/// a hand-edit gone wrong), it is moved aside and an empty store is returned
/// so the app always starts. The caller can surface `recovered_backup` to the
/// user so they know their old file is still on disk.
pub fn load() -> Result<Loaded> {
    let path = data_file()?;
    if !path.exists() {
        return Ok(Loaded {
            store: Store::default(),
            recovered_backup: None,
        });
    }
    let data = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    match serde_json::from_str::<Store>(&data) {
        Ok(store) => Ok(Loaded {
            store,
            recovered_backup: None,
        }),
        Err(_) => {
            let backup = backup_path(&path);
            // Best-effort: if the rename fails, fall back to starting empty anyway.
            let recovered_backup = fs::rename(&path, &backup).ok().map(|_| backup);
            Ok(Loaded {
                store: Store::default(),
                recovered_backup,
            })
        }
    }
}

/// Persist the store, pretty-printed so it's human-readable / git-diffable.
pub fn save(store: &Store) -> Result<()> {
    let path = data_file()?;
    let data = serde_json::to_string_pretty(store).context("serializing store")?;
    fs::write(&path, data).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
