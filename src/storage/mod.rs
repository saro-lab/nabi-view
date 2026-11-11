use std::{
    collections::HashSet,
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use tempfile::NamedTempFile;

use crate::core::library::{self, Entry, Kind};
use crate::core::settings::*;
const MAX_SETTINGS_BYTES: u64 = 128 * 1024;

pub struct Store {
    settings: PathBuf,
    database: PathBuf,
    cache: PathBuf,
    thumbnails_migrated: OnceLock<()>,
    // What this instance showed its user, which can differ from the disk
    // after another process changes a preference.
    baseline: Mutex<Settings>,
}

impl Store {
    #[cfg(all(target_os = "macos", not(test)))]
    pub(crate) fn state_directory(&self) -> &Path {
        self.database
            .parent()
            .expect("state database has a parent directory")
    }
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
        let cache = dirs::cache_dir().ok_or("User cache directory is unavailable.")?;
        Ok(Self::in_directories_with_cache(
            config.join(app),
            state.join(app),
            cache.join(app),
        ))
    }

    /// Compatibility constructor for isolated callers. Platform hosts should
    /// supply their OS cache directory with `in_directories_with_cache`.
    pub fn in_directories(config: PathBuf, state: PathBuf) -> Self {
        let cache = state.join("cache");
        Self::in_directories_with_cache(config, state, cache)
    }

    pub fn in_directories_with_cache(config: PathBuf, state: PathBuf, cache: PathBuf) -> Self {
        Self {
            settings: config.join("settings.json"),
            database: state.join("state.sqlite"),
            cache: cache.join("v1"),
            thumbnails_migrated: OnceLock::new(),
            baseline: Mutex::new(Settings::default()),
        }
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.cache.clone()
    }

    /// Move only our named cache files, preserving database keys and old JPEGs
    /// used by unavailable bookmarks. A failed copy leaves its old file intact.
    fn migrate_thumbnails(&self) -> Result<(), String> {
        if self.thumbnails_migrated.get().is_some() {
            return Ok(());
        }
        let legacy = self.database.with_file_name("thumbnails");
        let entries = match fs::read_dir(&legacy) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let _ = self.thumbnails_migrated.set(());
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        };
        let destination = self.thumbnail_dir();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
                || entry
                    .file_name()
                    .to_str()
                    .and_then(library::thumbnail_key_of_file)
                    .is_none()
            {
                continue;
            }
            fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
            let target = destination.join(entry.file_name());
            if !target.exists() {
                let mut input = File::open(entry.path()).map_err(|error| error.to_string())?;
                let mut temporary =
                    NamedTempFile::new_in(&destination).map_err(|error| error.to_string())?;
                std::io::copy(&mut input, &mut temporary).map_err(|error| error.to_string())?;
                temporary
                    .as_file()
                    .sync_all()
                    .map_err(|error| error.to_string())?;
                if let Err(error) = temporary.persist_noclobber(&target)
                    && error.error.kind() != ErrorKind::AlreadyExists
                {
                    return Err(error.error.to_string());
                }
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let _ = fs::remove_dir(legacy);
        let _ = self.thumbnails_migrated.set(());
        Ok(())
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
        settings.normalize();
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
        settings.normalize();
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
        settings.normalize();
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
        settings.normalize();
        settings.validate()?;
        Ok(settings)
    }

    pub fn export(&self, path: &Path, settings: &Settings) -> Result<(), String> {
        let mut settings = settings.clone();
        settings.normalize();
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
        let mut conn = Connection::open(&self.database).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        // Recent folders and bookmarks share one table; `kind` is Kind::code.
        // `image` is a path inside `path` (a folder or an archive), and
        // `thumbnail` a cache key.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS library (
                id INTEGER PRIMARY KEY,
                kind INTEGER NOT NULL,
                path BLOB NOT NULL,
                image BLOB,
                name TEXT,
                thumbnail TEXT,
                UNIQUE(kind, path)
            );
            CREATE TABLE IF NOT EXISTS recent_moves (
                id INTEGER PRIMARY KEY, path BLOB NOT NULL UNIQUE
            );",
        )
        .map_err(|e| e.to_string())?;
        migrate_recent_folders(&mut conn)?;
        Ok(conn)
    }

    /// Regenerable thumbnails live in the OS application cache, not user state.
    pub fn thumbnail_dir(&self) -> PathBuf {
        self.cache.join("thumbnails")
    }

    pub fn record_move(&self, path: &Path, limit: usize) -> Result<(), String> {
        if !RECENT_LIMIT.contains(&limit) {
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
        if !RECENT_LIMIT.contains(&limit) {
            return Err("Invalid recent move limit.".into());
        }
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(Vec::new());
        }
        let mut conn = self.connection()?;
        simplify_stored_paths(&mut conn, "recent_moves", "0")?;
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
        if !RECENT_LIMIT.contains(&limit) {
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

    /// Puts an opened folder at the top of the recent list. Its saved page and
    /// thumbnail are kept; `remember_page` changes the page.
    pub fn record_folder(&self, path: &Path, limit: usize) -> Result<(), String> {
        if !RECENT_LIMIT.contains(&limit) {
            return Err("Invalid recent item limit.".into());
        }
        let path = canonical_library_folder(path)?;
        // A selected file may grant access to that file without granting a
        // directory listing. Its parent remains a valid recent-list identity.
        if !accessible_library_folder(&path) {
            return Err("Recent history accepts folders only.".into());
        }
        let kind = Kind::Recent.code();
        let path = encode_path(&path);
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let (image, thumbnail): (Option<Vec<u8>>, Option<String>) = tx
            .query_row(
                "SELECT image, thumbnail FROM library WHERE kind = ?1 AND path = ?2",
                params![kind, path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        // A fresh rowid gives the exact open order even within one clock tick.
        tx.execute(
            "INSERT OR REPLACE INTO library (kind, path, image, thumbnail) VALUES (?1, ?2, ?3, ?4)",
            params![kind, path, image, thumbnail],
        )
        .map_err(|e| e.to_string())?;
        #[cfg(all(target_os = "macos", not(test)))]
        let evicted =
            {
                let mut query = tx.prepare(
                "SELECT path FROM library WHERE kind = ?1 ORDER BY id DESC LIMIT -1 OFFSET ?2"
            ).map_err(|e| e.to_string())?;
                let rows = query
                    .query_map(params![kind, limit as i64], |row| row.get::<_, Vec<u8>>(0))
                    .map_err(|e| e.to_string())?;
                rows.map(|row| row.map_err(|e| e.to_string()).and_then(decode_path))
                    .collect::<Result<Vec<_>, _>>()?
            };
        tx.execute(
            "DELETE FROM library WHERE kind = ?1 AND id IN (
                SELECT id FROM library WHERE kind = ?1 ORDER BY id DESC LIMIT -1 OFFSET ?2)",
            params![kind, limit as i64],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        #[cfg(all(target_os = "macos", not(test)))]
        for removed in evicted {
            // Do not sweep unrelated grants: another window may have just
            // selected a source and not painted/recorded its first image yet.
            self.forget_unused_mac_grants(&removed)?;
        }
        Ok(())
    }

    /// Saves `image` as the page its recent folder reopens at and shows as its
    /// thumbnail. Folders that are not in the recent list are left alone.
    pub fn remember_page(&self, image: &Path) -> Result<(), String> {
        let Some((folder, name)) = library::container(image) else {
            return Ok(());
        };
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        // A folder that has disappeared has nothing to update.
        let Ok(folder) = canonical_library_folder(&folder) else {
            return Ok(());
        };
        self.connection()?
            .execute(
                "UPDATE library SET image = ?3 WHERE kind = ?1 AND path = ?2",
                params![
                    Kind::Recent.code(),
                    encode_path(&folder),
                    encode_path(&name)
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Bookmarks the folder of `image`, with that image as its thumbnail, and
    /// returns the stored image path. A bookmarked folder is updated in place.
    pub fn add_bookmark(&self, image: &Path, name: &str) -> Result<PathBuf, String> {
        const INVALID: &str = "Bookmarks need an image inside a folder.";
        let Some((folder, file)) = library::container(image) else {
            return Err(INVALID.into());
        };
        let folder = canonical_library_folder(&folder)?;
        let image = folder.join(&file);
        // A page of an archive is bookmarked with the archive as its folder.
        if !accessible_library_folder(&folder)
            || !library::page_exists(&image)
            || !crate::core::media::is_supported(&image)
        {
            return Err(INVALID.into());
        }
        self.connection()?
            .execute(
                "INSERT INTO library (kind, path, image, name) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(kind, path) DO UPDATE SET image = excluded.image, name = excluded.name",
                params![
                    Kind::Bookmark.code(),
                    encode_path(&folder),
                    encode_path(&file),
                    library::clean_name(name)
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(image)
    }

    /// The title of the bookmark for `folder`, if it is bookmarked.
    pub fn bookmark_title(&self, folder: &Path) -> Result<Option<String>, String> {
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let Ok(folder) = canonical_library_folder(folder) else {
            return Ok(None);
        };
        let name: Option<Option<String>> = self
            .connection()?
            .query_row(
                "SELECT name FROM library WHERE kind = ?1 AND path = ?2",
                params![Kind::Bookmark.code(), encode_path(&folder)],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(name.map(|name| name.unwrap_or_else(|| library::folder_name(&folder))))
    }

    /// Renames a bookmark by its stored path; an empty name shows the folder name.
    pub fn rename_bookmark(&self, folder: &Path, name: &str) -> Result<(), String> {
        self.connection()?
            .execute(
                "UPDATE library SET name = ?3 WHERE kind = ?1 AND path = ?2",
                params![
                    Kind::Bookmark.code(),
                    encode_path(folder),
                    library::clean_name(name)
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Records the thumbnail written for an entry, so pruning keeps it.
    pub fn set_thumbnail(&self, kind: Kind, folder: &Path, key: &str) -> Result<(), String> {
        if !library::is_thumbnail_key(key) {
            return Err("Invalid thumbnail name.".into());
        }
        if self.database.try_exists().map_err(|e| e.to_string())? {
            self.connection()?
                .execute(
                    "UPDATE library SET thumbnail = ?3 WHERE kind = ?1 AND path = ?2",
                    params![kind.code(), encode_path(folder), key],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Recent folders, newest first, then bookmarks, newest first. Recent
    /// folders that can no longer be opened are removed; bookmarks stay and
    /// are marked unavailable until the user removes them.
    pub fn library(&self) -> Result<Vec<Entry>, String> {
        self.migrate_thumbnails()?;
        if !self.database.try_exists().map_err(|e| e.to_string())? {
            return Ok(Vec::new());
        }
        let mut conn = self.connection()?;
        simplify_stored_paths(&mut conn, "library", "kind")?;
        type Row = (
            i64,
            i64,
            Vec<u8>,
            Option<Vec<u8>>,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Row> = {
            let mut query = conn
                .prepare(
                    "SELECT id, kind, path, image, name, thumbnail FROM library
                     ORDER BY kind, id DESC",
                )
                .map_err(|e| e.to_string())?;
            query
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        };

        // Check access only when loading the list, not on every rendered frame.
        // An accessible folder without supported images is not a usable entry.
        let mut entries = Vec::with_capacity(rows.len());
        let mut unavailable = Vec::new();
        for (id, kind, path, image, name, thumbnail) in rows {
            let Some(kind) = Kind::from_code(kind) else {
                continue;
            };
            let folder = decode_path(path.clone())?;
            #[cfg(all(target_os = "macos", not(test)))]
            let protected = crate::platform::mac_access::has_grant(&folder);
            #[cfg(not(all(target_os = "macos", not(test))))]
            let protected = false;
            let saved = image
                .map(decode_path)
                .transpose()?
                .map(|image| folder.join(image))
                .filter(|image| protected || library::page_exists(image));
            // An archive whose file list cannot be read still opens by itself.
            let image = saved
                .or_else(|| library::first_image(&folder))
                .or_else(|| library::is_archive_file(&folder).then(|| folder.clone()));
            if image.is_none() && kind == Kind::Recent && !protected {
                unavailable.push((id, path));
                continue;
            }
            entries.push(Entry {
                kind,
                available: image
                    .as_ref()
                    .is_some_and(|path| !protected || library::page_exists(path)),
                folder,
                image,
                name,
                thumbnail: thumbnail.filter(|key| library::is_thumbnail_key(key)),
            });
        }
        // Do not hold a database write lock while checking filesystem access.
        if !unavailable.is_empty() {
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (id, path) in unavailable {
                tx.execute(
                    "DELETE FROM library WHERE id = ?1 AND path = ?2",
                    params![id, path],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        Ok(entries)
    }

    /// Recent folders only, newest first.
    #[cfg(test)]
    pub fn recent(&self) -> Result<Vec<PathBuf>, String> {
        Ok(self
            .library()?
            .into_iter()
            .filter(|entry| entry.kind == Kind::Recent)
            .map(|entry| entry.folder)
            .collect())
    }

    pub fn remove_entry(&self, kind: Kind, path: &Path) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            // Use the stored path verbatim: missing folders must also be removable.
            self.connection()?
                .execute(
                    "DELETE FROM library WHERE kind = ?1 AND path = ?2",
                    params![kind.code(), encode_path(path)],
                )
                .map_err(|e| e.to_string())?;
            #[cfg(all(target_os = "macos", not(test)))]
            self.forget_unused_mac_grants(path)?;
        }
        Ok(())
    }

    #[cfg(all(target_os = "macos", not(test)))]
    fn forget_unused_mac_grants(&self, removed: &Path) -> Result<(), String> {
        let retained = mac_grant_references(&self.database)
            .ok_or("Could not read saved document references; access records were preserved.")?;
        crate::platform::mac_access::forget(removed, &retained)
    }

    /// Clears the recent list and its thumbnails; bookmarks stay.
    pub fn clear_recent(&self) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            self.connection()?
                .execute("DELETE FROM library WHERE kind = ?1", [Kind::Recent.code()])
                .map_err(|e| e.to_string())?;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        self.forget_unused_mac_grants(Path::new("/"))?;
        self.prune_thumbnails(Duration::ZERO)
    }

    /// Deletes device-local state: recent folders and their thumbnails, move
    /// destinations and window size. Bookmarks, which the user curates, and
    /// portable preferences in settings.json are left untouched.
    pub fn clear_data(&self) -> Result<(), String> {
        if self.database.try_exists().map_err(|e| e.to_string())? {
            self.connection()?
                .execute_batch(&format!(
                    "BEGIN;
                    DELETE FROM library WHERE kind = {};
                    DELETE FROM recent_moves;
                    DROP TABLE IF EXISTS window_state;
                    COMMIT;",
                    Kind::Recent.code()
                ))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(all(target_os = "macos", not(test)))]
        self.forget_unused_mac_grants(Path::new("/"))?;
        self.prune_thumbnails(Duration::ZERO)
    }

    /// Deletes cached thumbnails that no entry uses. Files younger than `grace`
    /// stay, since another window may be about to record them.
    pub fn prune_thumbnails(&self, grace: Duration) -> Result<(), String> {
        self.migrate_thumbnails()?;
        let files = match fs::read_dir(self.thumbnail_dir()) {
            Ok(files) => files,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let used: HashSet<String> = if self.database.try_exists().map_err(|e| e.to_string())? {
            let conn = self.connection()?;
            let mut query = conn
                .prepare("SELECT thumbnail FROM library WHERE thumbnail IS NOT NULL")
                .map_err(|e| e.to_string())?;
            query
                .query_map([], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        } else {
            HashSet::new()
        };
        let now = SystemTime::now();
        for file in files.flatten() {
            let name = file.file_name();
            let name = name.to_string_lossy();
            // Only touch files this cache writes: thumbnails and interrupted temporary files.
            match library::thumbnail_key_of_file(&name) {
                Some(key) if used.contains(key) => continue,
                Some(_) => {}
                None if name.starts_with(".tmp") => {}
                None => continue,
            }
            if !grace.is_zero() {
                let age = file
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok());
                if age.is_none_or(|age| age < grace) {
                    continue;
                }
            }
            // Another window may have removed it first.
            let _ = fs::remove_file(file.path());
        }
        Ok(())
    }
}

/// Versions before bookmarks kept recent folders in their own table.
fn migrate_recent_folders(conn: &mut Connection) -> Result<(), String> {
    const LEGACY: &str =
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='recent_folders')";
    let legacy: bool = conn
        .query_row(LEGACY, [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if !legacy {
        return Ok(());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Another window may have finished the migration while this one waited.
    let legacy: bool = tx
        .query_row(LEGACY, [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if legacy {
        // Ascending inserts keep the newest-first order of the old row ids.
        tx.execute_batch(&format!(
            "INSERT OR IGNORE INTO library (kind, path)
                SELECT {}, path FROM recent_folders ORDER BY id;
             DROP TABLE recent_folders;",
            Kind::Recent.code()
        ))
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Rewrites history saved with Windows verbatim prefixes (`\\?\C:\…`) to the
/// spelling shown to the user, so display, removal and deduplication agree.
/// When the plain spelling is already stored, the newer record is kept.
/// Paths are unique within `group`, a column or a constant for one group.
fn simplify_stored_paths(conn: &mut Connection, table: &str, group: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let rows = {
        let mut query = conn
            .prepare(&format!("SELECT id, path, {group} FROM {table}"))
            .map_err(|e| e.to_string())?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let changed: Vec<_> = rows
        .into_iter()
        .filter_map(|(id, stored, scope)| {
            // Corrupt rows are reported by the caller's own read.
            let path = decode_path(stored.clone()).ok()?;
            let plain = encode_path(&crate::core::paths::simplified(path));
            (plain != stored).then_some((id, stored, plain, scope))
        })
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (id, stored, plain, scope) in changed {
        // Keep whichever record of the same folder was used most recently.
        tx.execute(
            &format!("DELETE FROM {table} WHERE {group} = ?3 AND path = ?1 AND id < ?2"),
            params![plain, id, scope],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            &format!("UPDATE OR IGNORE {table} SET path = ?1 WHERE id = ?2"),
            params![plain, id],
        )
        .map_err(|e| e.to_string())?;
        // Ignored above because a newer plain record exists: drop this one.
        tx.execute(
            &format!("DELETE FROM {table} WHERE id = ?1 AND path = ?2"),
            params![id, stored],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn canonical_library_folder(path: &Path) -> Result<PathBuf, String> {
    match crate::core::paths::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) => {
            #[cfg(all(target_os = "macos", not(test)))]
            if path.is_absolute() && crate::platform::mac_access::has_grant(path) {
                return Ok(path.to_owned());
            }
            Err(error.to_string())
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn mac_grant_references(database: &Path) -> Option<Vec<PathBuf>> {
    // Registry callback: raw read-only SQLite only, with no Store initialization
    // or permission callbacks. A busy/corrupt DB keeps the permission record.
    match database.try_exists() {
        Ok(false) => return Some(Vec::new()),
        Err(_) => return None,
        Ok(true) => {}
    }
    let result = (|| -> Result<Vec<PathBuf>, String> {
        let conn = Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_millis(50))
            .map_err(|e| e.to_string())?;
        let mut query = conn
            .prepare("SELECT DISTINCT path FROM library")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(decode_path(row.map_err(|e| e.to_string())?)?);
        }
        Ok(paths)
    })();
    result.ok()
}

fn accessible_library_folder(path: &Path) -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    if crate::platform::mac_access::has_grant(path) {
        return true;
    }
    path.is_dir() || library::is_archive_file(path)
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
