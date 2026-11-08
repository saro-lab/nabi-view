use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tempfile::NamedTempFile;

use crate::core::settings::*;
const MAX_SETTINGS_BYTES: u64 = 128 * 1024;

pub struct Store {
    settings: PathBuf,
    database: PathBuf,
    // What this instance showed its user, which can differ from the disk
    // after another process changes a preference.
    baseline: Mutex<Settings>,
}

impl Store {
    pub(crate) fn close_signal_path(&self) -> PathBuf {
        self.database.with_file_name("close-all.signal")
    }

    pub fn new() -> Result<Self, String> {
        let config = dirs::config_dir().ok_or("User configuration directory is unavailable.")?;
        #[cfg(target_os = "linux")]
        let state = dirs::state_dir();
        #[cfg(not(target_os = "linux"))]
        let state = dirs::data_local_dir();
        let state = state.ok_or("User state directory is unavailable.")?;
        let app = if cfg!(target_os = "linux") {
            "nabi-view"
        } else {
            "NabiView"
        };
        Ok(Self::in_directories(config.join(app), state.join(app)))
    }

    pub fn in_directories(config: PathBuf, state: PathBuf) -> Self {
        Self {
            settings: config.join("settings.json"),
            database: state.join("state.sqlite"),
            baseline: Mutex::new(Settings::default()),
        }
    }

    pub fn load(&self) -> Result<Settings, String> {
        let mut baseline = self.baseline.lock().map_err(|e| e.to_string())?;
        let loaded = self.read_current()?;
        baseline.clone_from(&loaded);
        Ok(loaded)
    }

    /// Window geometry belongs to this device and is excluded from portable settings.
    /// The initial read is read-only and does not create or migrate the state database.
    pub fn load_window_size(&self) -> Result<Option<[f32; 2]>, String> {
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_millis(30))
            .map_err(|e| e.to_string())?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='window_state')",
            [], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if !exists {
            return Ok(None);
        }
        let size: Option<[f32; 2]> = conn
            .query_row(
                "SELECT width, height FROM window_state WHERE id=1",
                [],
                |row| Ok([row.get::<_, f64>(0)? as f32, row.get::<_, f64>(1)? as f32]),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(size.filter(|&size| valid_window_size(size)))
    }

    pub fn save_window_size(&self, size: [f32; 2]) -> Result<(), String> {
        if !valid_window_size(size) {
            return Err("Invalid window dimensions.".into());
        }
        fs::create_dir_all(self.database.parent().unwrap()).map_err(|e| e.to_string())?;
        let conn = Connection::open(&self.database).map_err(|e| e.to_string())?;
        // Resize updates must not wait for another window's long database transaction.
        conn.busy_timeout(Duration::from_millis(30))
            .map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS window_state (
            id INTEGER PRIMARY KEY CHECK(id=1), width REAL NOT NULL, height REAL NOT NULL
        );",
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO window_state (id,width,height) VALUES (1,?1,?2)
             ON CONFLICT(id) DO UPDATE SET width=excluded.width, height=excluded.height",
            [size[0] as f64, size[1] as f64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_current(&self) -> Result<Settings, String> {
        match File::open(&self.settings) {
            Ok(file) => Self::read_settings(file),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        let mut settings = settings.clone();
        settings.normalize_shortcuts();
        self.write_preferences(&settings, |baseline| {
            let mut latest = self.read_current()?;
            latest.merge_changes(baseline, &settings);
            Ok(latest)
        })
    }

    /// An explicit shortcut edit replaces that list even when this window's
    /// baseline already matches it. Other settings still merge independently.
    pub fn save_shortcuts(&self, settings: &Settings) -> Result<(), String> {
        let mut settings = settings.clone();
        settings.normalize_shortcuts();
        self.write_preferences(&settings, |baseline| {
            let mut latest = self.read_current()?;
            latest.merge_changes(baseline, &settings);
            latest.shortcuts.clone_from(&settings.shortcuts);
            latest
                .keyboard_shortcuts
                .clone_from(&settings.keyboard_shortcuts);
            Ok(latest)
        })
    }

    pub fn replace_from_import(&self, settings: &Settings) -> Result<(), String> {
        let mut settings = settings.clone();
        settings.normalize_shortcuts();
        self.write_preferences(&settings, |_| Ok(settings.clone()))
    }

    fn write_preferences(
        &self,
        local: &Settings,
        combine: impl FnOnce(&Settings) -> Result<Settings, String>,
    ) -> Result<(), String> {
        local.validate()?;
        let mut baseline = self.baseline.lock().map_err(|e| e.to_string())?;
        fs::create_dir_all(self.settings.parent().unwrap()).map_err(|e| e.to_string())?;
        // Lock the stable companion file. Atomic replacement changes the
        // identity of settings.json, so locking the JSON itself cannot serialize writers.
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.settings.with_extension("lock"))
            .map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        let merged = combine(&baseline)?;
        self.export(&self.settings, &merged)?;
        baseline.clone_from(local);
        Ok(())
    }

    pub fn import(&self, path: &Path) -> Result<Settings, String> {
        Self::read_settings(File::open(path).map_err(|e| e.to_string())?)
    }

    fn read_settings(file: File) -> Result<Settings, String> {
        let mut json = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut json)
            .map_err(|e| e.to_string())?;
        if json.len() as u64 > MAX_SETTINGS_BYTES {
            return Err("Settings file exceeds the size limit.".into());
        }
        let mut settings: Settings = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
        settings.normalize_shortcuts();
        settings.validate()?;
        Ok(settings)
    }

    pub fn export(&self, path: &Path, settings: &Settings) -> Result<(), String> {
        let mut settings = settings.clone();
        settings.normalize_shortcuts();
        settings.validate()?;
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut temp, &settings).map_err(|e| e.to_string())?;
        temp.write_all(b"\n").map_err(|e| e.to_string())?;
        if temp.as_file().metadata().map_err(|e| e.to_string())?.len() > MAX_SETTINGS_BYTES {
            return Err("Settings data exceeds the size limit.".into());
        }
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }

    fn connection(&self) -> Result<Connection, String> {
        fs::create_dir_all(self.database.parent().unwrap()).map_err(|e| e.to_string())?;
        let conn = Connection::open(&self.database).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recent_folders (
                id INTEGER PRIMARY KEY,
                path BLOB NOT NULL UNIQUE,
                opened_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS recent_moves (
                id INTEGER PRIMARY KEY, path BLOB NOT NULL UNIQUE
            );",
        )
        .map_err(|e| e.to_string())?;
        Ok(conn)
    }

    pub fn record_move(&self, path: &Path, limit: usize) -> Result<(), String> {
        if !(1..=MAX_RECENT).contains(&limit) {
            return Err("Invalid recent move limit.".into());
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR REPLACE INTO recent_moves (path) VALUES (?1)",
            [encode_path(path)],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM recent_moves WHERE id IN (SELECT id FROM recent_moves ORDER BY id DESC LIMIT -1 OFFSET ?1)", [limit as i64])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn recent_moves(&self, limit: usize) -> Result<Vec<PathBuf>, String> {
        if !(1..=MAX_RECENT).contains(&limit) {
            return Err("Invalid recent move limit.".into());
        }
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(Vec::new());
        }
        let conn = self.connection()?;
        let mut query = conn
            .prepare("SELECT path FROM recent_moves ORDER BY id DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        query
            .query_map([limit as i64], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|e| e.to_string())?
            .map(|row| decode_path(row.map_err(|e| e.to_string())?))
            .collect()
    }

    pub fn trim_recent_moves(&self, limit: usize) -> Result<(), String> {
        if !(1..=MAX_RECENT).contains(&limit) {
            return Err("Invalid recent move limit.".into());
        }
        if self.database.try_exists().map_err(|e| e.to_string())? {
            self.connection()?.execute("DELETE FROM recent_moves WHERE id IN (SELECT id FROM recent_moves ORDER BY id DESC LIMIT -1 OFFSET ?1)", [limit as i64])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn remove_recent_move(&self, path: Option<&Path>) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            let conn = self.connection()?;
            if let Some(path) = path {
                conn.execute(
                    "DELETE FROM recent_moves WHERE path = ?1",
                    [encode_path(path)],
                )
            } else {
                conn.execute("DELETE FROM recent_moves", [])
            }
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn record_folder(&self, path: &Path, limit: usize) -> Result<(), String> {
        if !(1..=MAX_RECENT).contains(&limit) {
            return Err("Invalid recent item limit.".into());
        }
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        if !path.is_dir() {
            return Err("Recent history accepts folders only.".into());
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // A fresh rowid gives the exact open order even within one clock tick.
        tx.execute(
            "INSERT OR REPLACE INTO recent_folders (path, opened_at) VALUES (?1, unixepoch())",
            [encode_path(&path)],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM recent_folders WHERE id IN (
                       SELECT id FROM recent_folders ORDER BY id DESC LIMIT -1 OFFSET ?1)",
            [limit as i64],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn recent(&self) -> Result<Vec<PathBuf>, String> {
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(Vec::new());
        }
        let mut conn = self.connection()?;
        let mut q = conn
            .prepare("SELECT id, path FROM recent_folders ORDER BY id DESC")
            .map_err(|e| e.to_string())?;
        let paths = q
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let paths: Vec<_> = paths
            .map(|row| {
                let (id, bytes) = row.map_err(|e| e.to_string())?;
                Ok((id, decode_path(bytes)?))
            })
            .collect::<Result<_, String>>()?;
        drop(q);

        // Check access only when loading the list, not on every rendered frame.
        // An accessible folder without supported images is not a usable recent item.
        let mut recent = Vec::with_capacity(paths.len());
        let mut unavailable = Vec::new();
        for (id, path) in paths {
            if crate::core::media::images_in_folder(&path)
                .is_ok_and(|mut images| images.next().is_some())
            {
                recent.push(path);
            } else {
                unavailable.push((id, path));
            }
        }
        // Do not hold a database write lock while checking filesystem access.
        if !unavailable.is_empty() {
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (id, path) in unavailable {
                tx.execute(
                    "DELETE FROM recent_folders WHERE id = ?1 AND path = ?2",
                    rusqlite::params![id, encode_path(&path)],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        Ok(recent)
    }

    pub fn remove_recent(&self, path: &Path) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            // Use the stored path verbatim: missing folders must also be removable.
            self.connection()?
                .execute(
                    "DELETE FROM recent_folders WHERE path = ?1",
                    [encode_path(path)],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn clear_recent(&self) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            self.connection()?
                .execute("DELETE FROM recent_folders", [])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// Paths are machine-local state. Preserve filenames that cannot be represented by UTF-8.
#[cfg(unix)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(unix)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf, String> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}
#[cfg(windows)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
#[cfg(windows)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf, String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let pairs = bytes.chunks_exact(2);
    if !pairs.remainder().is_empty() {
        return Err("Stored folder path is corrupt.".into());
    }
    let units: Vec<_> = pairs.map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

#[cfg(test)]
mod tests;
