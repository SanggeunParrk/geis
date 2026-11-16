//! Carrying a user's data over from when the app was called mini.
//!
//! The app's data lives in a folder named by its identifier, and the identifier changed
//! with the name (`dev.mini.app` to `com.sanggeunpark.geis`). Without this, the first launch
//! under the new name would open onto an empty shelf while every world sat in the old
//! folder. So, once, before anything is opened: the old folder is moved to the new place
//! (a rename on the same disk - downloaded model weights can be gigabytes, and are not
//! copied), the database is renamed, and the absolute paths the database holds into the
//! old folder - a render's picture, a job's directory - are rewritten to point into the
//! new one.

use std::path::{Path, PathBuf};

/// The identifier the app had before.
pub const OLD_IDENTIFIER: &str = "dev.mini.app";
/// The database's name before.
const OLD_DATABASE: &str = "mini.sqlite";
/// The keychain service credentials were stored under before.
pub const OLD_SECRET_SERVICE: &str = "dev.mini.compute";

/// What was done, for the log.
#[derive(Debug, PartialEq, Eq)]
pub enum Carried {
    /// Nothing to carry: no old folder, or the new one already exists.
    Nothing,
    /// The old folder was moved here, and this many stored values were repointed.
    Moved { rewritten: usize },
}

/// Moves the old data folder to `data_dir` if there is one and `data_dir` does not exist.
///
/// The old folder is the sibling of the new one with the old identifier, which is where
/// the system put it: both are under the same Application Support.
pub fn carry_over(data_dir: &Path) -> Result<Carried, String> {
    let Some(parent) = data_dir.parent() else {
        return Ok(Carried::Nothing);
    };
    let old = parent.join(OLD_IDENTIFIER);
    carry_from(&old, data_dir)
}

fn carry_from(old: &Path, data_dir: &Path) -> Result<Carried, String> {
    // A new folder that exists is a user who has already started over, or a migration
    // already done: neither is to be overwritten.
    if data_dir.exists() || !old.is_dir() {
        return Ok(Carried::Nothing);
    }
    std::fs::rename(old, data_dir).map_err(|error| {
        format!(
            "could not move {} to {}: {error}",
            old.display(),
            data_dir.display()
        )
    })?;
    // The database and the two files SQLite keeps beside it in WAL mode, together: a
    // database renamed without its write-ahead log loses whatever was last written.
    for suffix in ["", "-wal", "-shm"] {
        let from = data_dir.join(format!("{OLD_DATABASE}{suffix}"));
        if from.exists() {
            let to = data_dir.join(format!("geis.sqlite{suffix}"));
            std::fs::rename(&from, &to).map_err(|error| error.to_string())?;
        }
    }
    let rewritten = repoint(&data_dir.join("geis.sqlite"), old, data_dir)?;
    Ok(Carried::Moved { rewritten })
}

/// Rewrites every text value in the database that holds a path into `old` to the same
/// path under `new`. Every text column of every table, rather than a list of the ones known
/// to hold paths today: a world's graph is a JSON document with paths anywhere inside it.
fn repoint(database: &Path, old: &Path, new: &Path) -> Result<usize, String> {
    if !database.exists() {
        return Ok(0);
    }
    let connection = rusqlite::Connection::open(database).map_err(|error| error.to_string())?;
    let (old, new) = (
        old.to_string_lossy().into_owned(),
        new.to_string_lossy().into_owned(),
    );
    let tables: Vec<String> = {
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .map_err(|error| error.to_string())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        names.filter_map(Result::ok).collect()
    };
    let mut rewritten = 0;
    for table in tables {
        let columns: Vec<String> = {
            let mut statement = connection
                .prepare(&format!(
                    "PRAGMA table_info(\"{}\")",
                    table.replace('"', "\"\"")
                ))
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })
                .map_err(|error| error.to_string())?;
            rows.filter_map(Result::ok)
                .filter(|(_, kind)| {
                    let kind = kind.to_ascii_uppercase();
                    kind.is_empty()
                        || kind.contains("TEXT")
                        || kind.contains("CHAR")
                        || kind.contains("CLOB")
                })
                .map(|(name, _)| name)
                .collect()
        };
        for column in columns {
            let (table, column) = (table.replace('"', "\"\""), column.replace('"', "\"\""));
            rewritten += connection
                .execute(
                    &format!(
                        "UPDATE \"{table}\" SET \"{column}\" = replace(\"{column}\", ?1, ?2) \
                         WHERE typeof(\"{column}\") = 'text' AND instr(\"{column}\", ?1) > 0"
                    ),
                    [&old, &new],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(rewritten)
}

/// A credential, under the keychain service it is stored under now or the one it was
/// stored under before the rename.
pub fn read_secret(service: &str, reference: &str) -> Option<String> {
    for service in [service, OLD_SECRET_SERVICE] {
        if let Ok(value) =
            keyring::Entry::new(service, reference).and_then(|entry| entry.get_password())
        {
            return Some(value);
        }
    }
    None
}

/// A path to read from an environment variable, by its new name or the one before.
pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(format!("GEIS_{name}"))
        .or_else(|| std::env::var_os(format!("MINI_{name}")))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn folder() -> Folder {
        let dir = std::env::temp_dir().join(format!("geis-legacy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Folder(dir)
    }

    #[test]
    fn the_old_folder_moves_and_the_paths_into_it_follow() {
        let root = folder();
        let old = root.0.join(OLD_IDENTIFIER);
        let new = root.0.join("com.sanggeunpark.geis");
        std::fs::create_dir_all(old.join("renders")).unwrap();
        std::fs::write(old.join("renders/a.png"), b"png").unwrap();
        {
            let db = rusqlite::Connection::open(old.join(OLD_DATABASE)).unwrap();
            db.execute_batch("CREATE TABLE settings (key TEXT, value TEXT); CREATE TABLE jobs (id INTEGER, dir TEXT, n INTEGER);").unwrap();
            let picture = format!(
                r#"{{"nodes":[{{"type":"image","path":"{}/renders/a.png"}}]}}"#,
                old.display()
            );
            db.execute("INSERT INTO settings VALUES ('graph', ?1)", [&picture])
                .unwrap();
            db.execute("INSERT INTO settings VALUES ('other', 'untouched')", [])
                .unwrap();
            db.execute(
                "INSERT INTO jobs VALUES (1, ?1, 5)",
                [format!("{}/jobs/1", old.display())],
            )
            .unwrap();
        }

        let carried = carry_from(&old, &new).unwrap();
        assert_eq!(carried, Carried::Moved { rewritten: 2 });
        assert!(!old.exists());
        assert!(new.join("renders/a.png").exists());
        assert!(new.join("geis.sqlite").exists());
        let db = rusqlite::Connection::open(new.join("geis.sqlite")).unwrap();
        let graph: String = db
            .query_row(
                "SELECT value FROM settings WHERE key = 'graph'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            graph.contains(&format!("{}/renders/a.png", new.display())),
            "{graph}"
        );
        assert!(!graph.contains(OLD_IDENTIFIER));
        let dir: String = db
            .query_row("SELECT dir FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(dir, format!("{}/jobs/1", new.display()));
    }

    #[test]
    fn a_new_folder_that_exists_is_never_overwritten() {
        let root = folder();
        let old = root.0.join(OLD_IDENTIFIER);
        let new = root.0.join("com.sanggeunpark.geis");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        assert_eq!(carry_from(&old, &new).unwrap(), Carried::Nothing);
        assert!(old.exists());
        // And with nothing old, nothing happens.
        let fresh = root.0.join("fresh");
        assert_eq!(
            carry_from(&root.0.join("absent"), &fresh).unwrap(),
            Carried::Nothing
        );
        assert!(!fresh.exists());
    }
}
