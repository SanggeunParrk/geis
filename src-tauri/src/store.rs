//! Persistence: one SQLite file for the whole application.
//!
//! Worlds, providers, jobs, and artifacts live in one database rather than a directory of
//! session files. That is the deliberate break from PyMOL's file-per-session model - a
//! job needs to be attributable to a world, and a world needs the history of the jobs
//! that produced it, which a bag of independent files cannot express.

use crate::compute::job::{JobSpec, JobState};
use crate::compute::provider::{Artifact, ProviderConfig, ProviderRecord, ProviderStatus};
use crate::compute::HostProbe;
use crate::util;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Store {
    connection: Mutex<Connection>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored json for {what} is unreadable: {source}")]
    Json {
        what: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("no such {kind}: {id}")]
    NotFound { kind: &'static str, id: String },
}

/// A job as the UI lists it. `spec` is kept so a job can be re-run or resumed without the
/// original request, and so a completed job still knows what it was asked to do after the
/// manifest that produced it has been edited.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub model_name: String,
    pub provider_id: String,
    pub task: String,
    pub state: JobState,
    pub world_id: Option<String>,
    pub local_dir: PathBuf,
    pub external_id: Option<String>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub spec: JobSpec,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let connection = Connection::open(path)?;
        // Another connection to the same file - a second window's process, a backup tool
        // - holding the write lock for a moment is a reason to wait, not to fail a save
        // with "database is locked".
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL so a long-running write never blocks the UI's reads.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let store = Self {
            connection: Mutex::new(Connection::open_in_memory()?),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS providers (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                kind       TEXT NOT NULL,
                config     TEXT NOT NULL,
                enabled    INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                probe      TEXT,
                status     TEXT
            );

            CREATE TABLE IF NOT EXISTS worlds (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                thumbnail  TEXT,
                state      TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                model_id    TEXT NOT NULL,
                model_name  TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                task        TEXT NOT NULL,
                state       TEXT NOT NULL,
                world_id    TEXT,
                local_dir   TEXT NOT NULL,
                external_id TEXT,
                exit_code   INTEGER,
                error       TEXT,
                created_at  TEXT NOT NULL,
                started_at  TEXT,
                finished_at TEXT,
                spec        TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS jobs_by_state ON jobs(state, created_at DESC);
            CREATE INDEX IF NOT EXISTS jobs_by_world ON jobs(world_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS artifacts (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id     TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
                name       TEXT NOT NULL,
                kind       TEXT NOT NULL,
                path       TEXT NOT NULL,
                bytes      INTEGER NOT NULL DEFAULT 0,
                primary_output INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS artifacts_by_job ON artifacts(job_id);
            "#,
        )?;
        Ok(())
    }

    // --- settings -----------------------------------------------------------

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        connection.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let connection = self.connection.lock().unwrap();
        Ok(connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// A setting that holds a JSON list, read one entry at a time.
    ///
    /// Read whole, one entry this build cannot parse - a field a newer build added, a
    /// variant an older one never had - fails the list, the caller sees nothing saved,
    /// and the next write replaces everything with what it believes is the full set. So
    /// the entries that do parse are returned, and whenever anything was left out the
    /// text as it stood is kept under `<key>.unreadable`, where a later build - or a
    /// person - can still get at it.
    pub fn list_setting<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Vec<T>, StoreError> {
        let Some(text) = self.setting(key)? else {
            return Ok(Vec::new());
        };
        let entries: Vec<serde_json::Value> = match serde_json::from_str(&text) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("[store] {key} is not a list ({error}); kept as {key}.unreadable");
                self.set_setting(&format!("{key}.unreadable"), &text)?;
                return Ok(Vec::new());
            }
        };
        let total = entries.len();
        let read: Vec<T> = entries
            .into_iter()
            .filter_map(|entry| match serde_json::from_value(entry) {
                Ok(value) => Some(value),
                Err(error) => {
                    eprintln!("[store] an entry of {key} could not be read: {error}");
                    None
                }
            })
            .collect();
        if read.len() != total {
            self.set_setting(&format!("{key}.unreadable"), &text)?;
        }
        Ok(read)
    }

    /// A setting that holds one JSON object. Unreadable text is kept aside, as above,
    /// before the default takes its place.
    pub fn object_setting<T: serde::de::DeserializeOwned + Default>(
        &self,
        key: &str,
    ) -> Result<T, StoreError> {
        let Some(text) = self.setting(key)? else {
            return Ok(T::default());
        };
        match serde_json::from_str(&text) {
            Ok(value) => Ok(value),
            Err(error) => {
                eprintln!("[store] {key} could not be read ({error}); kept as {key}.unreadable");
                self.set_setting(&format!("{key}.unreadable"), &text)?;
                Ok(T::default())
            }
        }
    }

    // --- providers ----------------------------------------------------------

    pub fn upsert_provider(&self, record: &ProviderRecord) -> Result<(), StoreError> {
        let config = serde_json::to_string(&record.config).map_err(|source| StoreError::Json {
            what: "provider config".into(),
            source,
        })?;
        let probe = record
            .probe
            .as_ref()
            .and_then(|probe| serde_json::to_string(probe).ok());
        let status = serde_json::to_string(&record.status).ok();
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT INTO providers(id, name, kind, config, enabled, created_at, probe, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name, kind = excluded.kind, config = excluded.config,
                enabled = excluded.enabled, probe = excluded.probe, status = excluded.status",
            params![
                record.id,
                record.name,
                record.config.kind().as_str(),
                config,
                record.enabled as i32,
                record.created_at,
                probe,
                status,
            ],
        )?;
        Ok(())
    }

    pub fn providers(&self) -> Result<Vec<ProviderRecord>, StoreError> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, name, config, enabled, created_at, probe, status
             FROM providers ORDER BY created_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;

        let mut providers = Vec::new();
        for row in rows {
            let (id, name, config, enabled, created_at, probe, status) = row?;
            let config: ProviderConfig =
                serde_json::from_str(&config).map_err(|source| StoreError::Json {
                    what: format!("provider {id} config"),
                    source,
                })?;
            providers.push(ProviderRecord {
                id,
                name,
                config,
                enabled: enabled != 0,
                created_at,
                probe: probe.and_then(|json| serde_json::from_str::<HostProbe>(&json).ok()),
                status: status
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or(ProviderStatus::Unknown),
            });
        }
        Ok(providers)
    }

    pub fn provider(&self, id: &str) -> Result<ProviderRecord, StoreError> {
        self.providers()?
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| StoreError::NotFound {
                kind: "provider",
                id: id.to_string(),
            })
    }

    pub fn delete_provider(&self, id: &str) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        connection.execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_provider_probe(
        &self,
        id: &str,
        probe: Option<&HostProbe>,
        status: &ProviderStatus,
    ) -> Result<(), StoreError> {
        let probe = probe.and_then(|probe| serde_json::to_string(probe).ok());
        let status = serde_json::to_string(status).ok();
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "UPDATE providers SET probe = ?2, status = ?3 WHERE id = ?1",
            params![id, probe, status],
        )?;
        Ok(())
    }

    // --- jobs ---------------------------------------------------------------

    pub fn insert_job(&self, spec: &JobSpec) -> Result<JobRecord, StoreError> {
        let record = JobRecord {
            id: spec.id.clone(),
            name: spec.name.clone(),
            model_id: spec.model_id.clone(),
            model_name: spec.model_name.clone(),
            provider_id: spec.provider_id.clone(),
            task: serde_json::to_value(spec.task)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "custom".into()),
            state: JobState::Queued,
            world_id: spec.world_id.clone(),
            local_dir: spec.local_dir.clone(),
            external_id: None,
            exit_code: None,
            error: None,
            created_at: util::now_iso(),
            started_at: None,
            finished_at: None,
            spec: spec.clone(),
        };
        let spec_json = serde_json::to_string(spec).map_err(|source| StoreError::Json {
            what: "job spec".into(),
            source,
        })?;
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT INTO jobs(id, name, model_id, model_name, provider_id, task, state,
                              world_id, local_dir, created_at, spec)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                record.id,
                record.name,
                record.model_id,
                record.model_name,
                record.provider_id,
                record.task,
                record.state.as_str(),
                record.world_id,
                record.local_dir.to_string_lossy(),
                record.created_at,
                spec_json,
            ],
        )?;
        Ok(record)
    }

    pub fn set_job_state(&self, id: &str, state: JobState) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        let timestamp = util::now_iso();
        match state {
            JobState::Running => connection.execute(
                "UPDATE jobs SET state = ?2, started_at = COALESCE(started_at, ?3) WHERE id = ?1",
                params![id, state.as_str(), timestamp],
            )?,
            _ if state.is_terminal() => connection.execute(
                "UPDATE jobs SET state = ?2, finished_at = ?3 WHERE id = ?1",
                params![id, state.as_str(), timestamp],
            )?,
            _ => connection.execute(
                "UPDATE jobs SET state = ?2 WHERE id = ?1",
                params![id, state.as_str()],
            )?,
        };
        Ok(())
    }

    pub fn finish_job(
        &self,
        id: &str,
        state: JobState,
        exit_code: Option<i32>,
        error: Option<&str>,
        external_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "UPDATE jobs SET state = ?2, exit_code = ?3, error = ?4,
                             external_id = COALESCE(?5, external_id), finished_at = ?6
             WHERE id = ?1",
            params![
                id,
                state.as_str(),
                exit_code,
                error,
                external_id,
                util::now_iso()
            ],
        )?;
        Ok(())
    }

    pub fn job(&self, id: &str) -> Result<JobRecord, StoreError> {
        let connection = self.connection.lock().unwrap();
        let record = connection
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1"),
                params![id],
                read_job,
            )
            .optional()?;
        record.ok_or_else(|| StoreError::NotFound {
            kind: "job",
            id: id.to_string(),
        })
    }

    /// Newest first.
    ///
    /// `created_at` is to the second, so jobs submitted together tie on it; the row id,
    /// which only grows, breaks the tie in the order they were made. Without it the
    /// "last install or removal" read off this list could be either of two.
    ///
    /// A row whose spec this build cannot read - written by a build whose `JobSpec` had
    /// different fields - is left out rather than failing the whole history.
    pub fn jobs(&self, world_id: Option<&str>, limit: u32) -> Result<Vec<JobRecord>, StoreError> {
        let connection = self.connection.lock().unwrap();
        let mut jobs = Vec::new();
        let mut keep = |row: rusqlite::Result<JobRecord>| -> Result<(), StoreError> {
            match row {
                Ok(job) => jobs.push(job),
                Err(rusqlite::Error::FromSqlConversionFailure(_, _, error)) => {
                    eprintln!("[store] a job could not be read and was left out: {error}");
                }
                Err(error) => return Err(error.into()),
            }
            Ok(())
        };
        match world_id {
            Some(world_id) => {
                let mut statement = connection.prepare(&format!(
                    "SELECT {JOB_COLUMNS} FROM jobs WHERE world_id = ?1
                     ORDER BY created_at DESC, rowid DESC LIMIT ?2"
                ))?;
                for row in statement.query_map(params![world_id, limit], read_job)? {
                    keep(row)?;
                }
            }
            None => {
                let mut statement = connection.prepare(&format!(
                    "SELECT {JOB_COLUMNS} FROM jobs ORDER BY created_at DESC, rowid DESC LIMIT ?1"
                ))?;
                for row in statement.query_map(params![limit], read_job)? {
                    keep(row)?;
                }
            }
        }
        Ok(jobs)
    }

    /// Jobs left mid-flight by a crash or a quit. Nothing is running after a restart, so
    /// leaving them as "running" would show a job that will never finish; they are marked
    /// failed with an explanation instead.
    pub fn reap_interrupted_jobs(&self) -> Result<usize, StoreError> {
        let connection = self.connection.lock().unwrap();
        let count = connection.execute(
            "UPDATE jobs SET state = 'failed', finished_at = ?1,
                             error = 'interrupted when the application exited'
             WHERE state IN ('queued', 'staging', 'pending', 'running', 'fetching')",
            params![util::now_iso()],
        )?;
        Ok(count)
    }

    // --- artifacts ----------------------------------------------------------

    pub fn replace_artifacts(
        &self,
        job_id: &str,
        artifacts: &[Artifact],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM artifacts WHERE job_id = ?1", params![job_id])?;
        for artifact in artifacts {
            let kind = serde_json::to_value(artifact.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "any".into());
            transaction.execute(
                "INSERT INTO artifacts(job_id, name, kind, path, bytes, primary_output, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    job_id,
                    artifact.name,
                    kind,
                    artifact.path.to_string_lossy(),
                    artifact.bytes as i64,
                    artifact.primary as i32,
                    util::now_iso(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn artifacts(&self, job_id: &str) -> Result<Vec<StoredArtifact>, StoreError> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT name, kind, path, bytes, primary_output FROM artifacts
             WHERE job_id = ?1 ORDER BY primary_output DESC, path",
        )?;
        let rows = statement.query_map(params![job_id], |row| {
            Ok(StoredArtifact {
                name: row.get(0)?,
                kind: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                bytes: row.get::<_, i64>(3)? as u64,
                primary: row.get::<_, i32>(4)? != 0,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredArtifact {
    pub name: String,
    pub kind: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub primary: bool,
}

const JOB_COLUMNS: &str = "id, name, model_id, model_name, provider_id, task, state, world_id, \
     local_dir, external_id, exit_code, error, created_at, started_at, finished_at, spec";

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let state: String = row.get(6)?;
    let spec: String = row.get(15)?;
    Ok(JobRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        model_id: row.get(2)?,
        model_name: row.get(3)?,
        provider_id: row.get(4)?,
        task: row.get(5)?,
        state: JobState::parse(&state).unwrap_or(JobState::Failed),
        world_id: row.get(7)?,
        local_dir: PathBuf::from(row.get::<_, String>(8)?),
        external_id: row.get(9)?,
        exit_code: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
        spec: serde_json::from_str(&spec).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                15,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::provider::{ExecOptions, LocalConfig};
    use crate::models::manifest::Requirements;
    use crate::models::Task;
    use std::collections::BTreeMap;

    fn spec(id: &str) -> JobSpec {
        JobSpec {
            id: id.to_string(),
            name: "test run".into(),
            model_id: "boltz2".into(),
            model_name: "Boltz-2".into(),
            task: Task::StructurePrediction,
            provider_id: "local".into(),
            world_id: Some("world-1".into()),
            local_dir: PathBuf::from("/tmp/geis/test"),
            steps: vec![],
            scalars: BTreeMap::new(),
            files: BTreeMap::new(),
            outputs: BTreeMap::new(),
            requirements: Requirements::default(),
            http: None,
        }
    }

    #[test]
    fn jobs_round_trip_with_their_spec() {
        let store = Store::open_in_memory().unwrap();
        store.insert_job(&spec("job-1")).unwrap();
        let loaded = store.job("job-1").unwrap();
        assert_eq!(loaded.state, JobState::Queued);
        assert_eq!(loaded.spec.model_id, "boltz2");
        assert_eq!(loaded.world_id.as_deref(), Some("world-1"));
    }

    #[test]
    fn finishing_a_job_records_why() {
        let store = Store::open_in_memory().unwrap();
        store.insert_job(&spec("job-2")).unwrap();
        store.set_job_state("job-2", JobState::Running).unwrap();
        store
            .finish_job("job-2", JobState::Failed, Some(2), Some("boom"), None)
            .unwrap();
        let loaded = store.job("job-2").unwrap();
        assert_eq!(loaded.state, JobState::Failed);
        assert_eq!(loaded.exit_code, Some(2));
        assert_eq!(loaded.error.as_deref(), Some("boom"));
        assert!(loaded.started_at.is_some());
        assert!(loaded.finished_at.is_some());
    }

    #[test]
    fn interrupted_jobs_do_not_survive_a_restart_as_running() {
        let store = Store::open_in_memory().unwrap();
        store.insert_job(&spec("job-3")).unwrap();
        store.set_job_state("job-3", JobState::Running).unwrap();
        assert_eq!(store.reap_interrupted_jobs().unwrap(), 1);
        let loaded = store.job("job-3").unwrap();
        assert_eq!(loaded.state, JobState::Failed);
        assert!(loaded.error.unwrap().contains("interrupted"));
        // Idempotent: a second restart has nothing left to reap.
        assert_eq!(store.reap_interrupted_jobs().unwrap(), 0);
    }

    #[test]
    fn providers_round_trip_through_their_tagged_config() {
        let store = Store::open_in_memory().unwrap();
        let record = ProviderRecord {
            id: "local".into(),
            name: "This Mac".into(),
            config: ProviderConfig::Local(LocalConfig {
                exec: ExecOptions {
                    weights_dir: Some("/data/weights".into()),
                    max_parallel: 2,
                    ..Default::default()
                },
            }),
            enabled: true,
            created_at: util::now_iso(),
            probe: None,
            status: ProviderStatus::Unknown,
        };
        store.upsert_provider(&record).unwrap();
        let loaded = store.provider("local").unwrap();
        assert_eq!(loaded.name, "This Mac");
        assert_eq!(loaded.config.max_parallel(), 2);
        assert_eq!(
            loaded.config.exec_options().unwrap().weights_dir.as_deref(),
            Some("/data/weights")
        );
    }

    fn scratch_db() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("geis-store-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested/geis.sqlite");
        (dir, path)
    }

    #[test]
    fn a_fresh_database_is_created_and_reopened_without_losing_anything() {
        let (dir, path) = scratch_db();
        {
            let store = Store::open(&path).unwrap();
            store.set_setting("k", "v").unwrap();
            store.insert_job(&spec("job-1")).unwrap();
        }
        // Opening again runs the migration again, which must change nothing.
        let store = Store::open(&path).unwrap();
        assert_eq!(store.setting("k").unwrap().as_deref(), Some("v"));
        assert_eq!(store.jobs(None, 10).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_database_from_before_some_tables_existed_is_brought_up_to_date() {
        let (dir, path) = scratch_db();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings VALUES ('worlds', '[]');",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.setting("worlds").unwrap().as_deref(), Some("[]"));
        store.insert_job(&spec("job-1")).unwrap();
        assert!(store.providers().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_unreadable_job_does_not_hide_the_history() {
        let store = Store::open_in_memory().unwrap();
        store.insert_job(&spec("good")).unwrap();
        store.insert_job(&spec("bad")).unwrap();
        // As a build with a different JobSpec would have left it.
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE jobs SET spec = '{\"id\":\"bad\"}', state = 'warp' WHERE id = 'bad'",
                [],
            )
            .unwrap();
        let jobs = store.jobs(None, 10).unwrap();
        assert_eq!(
            jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec!["good"]
        );
        assert!(
            store.job("bad").is_err(),
            "asked for by id, it says it cannot be read"
        );
    }

    #[test]
    fn jobs_made_in_the_same_second_list_newest_first() {
        let store = Store::open_in_memory().unwrap();
        for id in ["first", "second", "third"] {
            store.insert_job(&spec(id)).unwrap();
        }
        store
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE jobs SET created_at = '2026-01-01T00:00:00Z'", [])
            .unwrap();
        let ids: Vec<String> = store
            .jobs(None, 10)
            .unwrap()
            .into_iter()
            .map(|job| job.id)
            .collect();
        assert_eq!(ids, vec!["third", "second", "first"]);
        let ids: Vec<String> = store
            .jobs(Some("world-1"), 2)
            .unwrap()
            .into_iter()
            .map(|job| job.id)
            .collect();
        assert_eq!(ids, vec!["third", "second"]);
        assert!(store.jobs(Some("other"), 10).unwrap().is_empty());
    }

    #[test]
    fn text_is_stored_exactly_as_given() {
        let store = Store::open_in_memory().unwrap();
        for (key, value) in [
            ("chat:it's", "she said \"hi\"; DROP TABLE settings; --"),
            ("view:단백질", "구조 🧬 \u{0} after a nul"),
            ("layout:", ""),
        ] {
            store.set_setting(key, value).unwrap();
            assert_eq!(store.setting(key).unwrap().as_deref(), Some(value));
        }
        let mut job = spec("job-'quote'");
        job.name = "Ünïcode \"run\" 'x'".into();
        store.insert_job(&job).unwrap();
        assert_eq!(store.job("job-'quote'").unwrap().name, job.name);
        store.delete_setting("chat:it's").unwrap();
        assert!(store.setting("chat:it's").unwrap().is_none());
        assert!(
            store.setting("view:단백질").unwrap().is_some(),
            "only that key went"
        );
    }

    #[test]
    fn two_connections_and_many_threads_can_write_at_once() {
        let (dir, path) = scratch_db();
        let first = std::sync::Arc::new(Store::open(&path).unwrap());
        let second = std::sync::Arc::new(Store::open(&path).unwrap());
        let handles: Vec<_> = (0..8)
            .map(|thread| {
                let store = if thread % 2 == 0 {
                    first.clone()
                } else {
                    second.clone()
                };
                std::thread::spawn(move || {
                    for index in 0..25 {
                        store
                            .set_setting(&format!("t{thread}-{index}"), "x")
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let count: i64 = first
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 200);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn artifacts_are_replaced_whole_and_go_with_their_job() {
        use crate::models::OutputKind;
        let store = Store::open_in_memory().unwrap();
        store.insert_job(&spec("job-1")).unwrap();
        let artifact = |path: &str, primary: bool| Artifact {
            name: "x".into(),
            kind: OutputKind::Structure,
            path: PathBuf::from(path),
            bytes: 3,
            primary,
        };
        store
            .replace_artifacts(
                "job-1",
                &[artifact("/b.cif", false), artifact("/a.cif", true)],
            )
            .unwrap();
        store
            .replace_artifacts(
                "job-1",
                &[artifact("/c.cif", false), artifact("/a.cif", true)],
            )
            .unwrap();
        let stored = store.artifacts("job-1").unwrap();
        assert_eq!(stored.len(), 2, "the second set replaced the first");
        assert!(stored[0].primary);
        assert_eq!(stored[0].kind, "structure");
        assert!(store.artifacts("unknown").unwrap().is_empty());
    }

    #[test]
    fn reaping_leaves_finished_jobs_alone() {
        let store = Store::open_in_memory().unwrap();
        for (id, state) in [
            ("q", JobState::Queued),
            ("p", JobState::Pending),
            ("s", JobState::Succeeded),
            ("c", JobState::Cancelled),
        ] {
            store.insert_job(&spec(id)).unwrap();
            store.set_job_state(id, state).unwrap();
        }
        assert_eq!(store.reap_interrupted_jobs().unwrap(), 2);
        assert_eq!(store.job("s").unwrap().state, JobState::Succeeded);
        assert_eq!(store.job("c").unwrap().state, JobState::Cancelled);
        assert_eq!(store.job("q").unwrap().state, JobState::Failed);
    }

    #[test]
    fn one_unreadable_entry_does_not_empty_the_list() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Named {
            name: String,
        }
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_setting::<Named>("things").unwrap().is_empty());

        let saved = r#"[{"name":"a"},{"nom":"from another build"},{"name":"c"}]"#;
        store.set_setting("things", saved).unwrap();
        let read: Vec<Named> = store.list_setting("things").unwrap();
        assert_eq!(
            read,
            vec![Named { name: "a".into() }, Named { name: "c".into() }]
        );
        // What could not be read is still there to be recovered.
        assert_eq!(
            store.setting("things.unreadable").unwrap().as_deref(),
            Some(saved)
        );

        // Text that is not a list at all is kept the same way, and nothing is invented.
        store.set_setting("things", "{broken").unwrap();
        assert!(store.list_setting::<Named>("things").unwrap().is_empty());
        assert_eq!(
            store.setting("things.unreadable").unwrap().as_deref(),
            Some("{broken")
        );
    }
}
