//! Job scheduling: what runs, where, how many at once, and who gets told about it.
//!
//! The queue is the only thing that starts work. Everything else - the UI, the world
//! system, a future batch runner - submits a `JobSpec` and then only listens to events.

use super::exec::{CancelToken, LogChunk, LogSink};
use super::job::{JobSpec, JobState};
use super::probe::HostProbe;
use super::provider::{self, ProviderError, ProviderStatus};
use crate::store::{JobRecord, Store};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, Semaphore};

/// How the queue reports progress. Implemented by the app layer over Tauri events; kept
/// as a trait so the queue can be driven in tests with no windowing system present.
pub trait Events: Send + Sync + 'static {
    fn job_changed(&self, record: &JobRecord);
    fn job_log(&self, chunk: LogChunk);
    fn provider_changed(&self, provider_id: &str);
}

pub struct Queue {
    store: Arc<Store>,
    events: Arc<dyn Events>,
    /// Held explicitly rather than relying on `tokio::spawn` finding an ambient runtime.
    /// Work is started from places that have none - the app's setup hook, and synchronous
    /// IPC command handlers - and a bare `tokio::spawn` there aborts the process.
    runtime: Handle,
    log_tx: mpsc::UnboundedSender<LogChunk>,
    running: Mutex<HashMap<String, CancelToken>>,
    /// One semaphore per provider, sized by its `max_parallel`. Two jobs sharing a single
    /// GPU do not run twice as fast; they run out of memory.
    permits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error("provider {0} is disabled")]
    Disabled(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Queue {
    pub fn new(store: Arc<Store>, events: Arc<dyn Events>, runtime: Handle) -> Arc<Self> {
        let (log_tx, mut log_rx) = mpsc::unbounded_channel::<LogChunk>();
        let queue = Arc::new(Self {
            store,
            events: events.clone(),
            runtime: runtime.clone(),
            log_tx,
            running: Mutex::new(HashMap::new()),
            permits: Mutex::new(HashMap::new()),
        });
        // Output arrives from blocking pty reader threads; this task is the single place
        // it crosses into the UI, so a chatty job cannot block the reader.
        runtime.spawn(async move {
            while let Some(chunk) = log_rx.recv().await {
                events.job_log(chunk);
            }
        });
        queue
    }

    fn permit_for(&self, provider_id: &str, max_parallel: usize) -> Arc<Semaphore> {
        let mut permits = self.permits.lock().unwrap();
        permits
            .entry(provider_id.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(max_parallel)))
            .clone()
    }

    /// Record a job and start it. Returns as soon as it is queued - the caller does not
    /// wait for a prediction to finish.
    pub fn submit(self: &Arc<Self>, spec: JobSpec) -> Result<JobRecord, QueueError> {
        let provider_record = self.store.provider(&spec.provider_id)?;
        if !provider_record.enabled {
            return Err(QueueError::Disabled(provider_record.name));
        }
        let record = self.store.insert_job(&spec)?;
        self.events.job_changed(&record);

        // The token exists from the moment the job does, not from the moment it starts.
        // A job waiting behind another for the provider's only slot is exactly the one a
        // user changes their mind about, and a token made on start cannot be found then.
        let cancel = CancelToken::new();
        self.running
            .lock()
            .unwrap()
            .insert(spec.id.clone(), cancel.clone());

        let queue = Arc::clone(self);
        let max_parallel = provider_record.config.max_parallel();
        self.runtime.spawn(async move {
            let permit = queue
                .permit_for(&spec.provider_id, max_parallel)
                .acquire_owned()
                .await;
            if permit.is_err() {
                queue.fail(&spec.id, "the provider's queue was closed");
                return;
            }
            // Run as a task of its own so that a panic anywhere below - a provider, a
            // parser, a slice - is reported here as a failed job. Left to unwind this
            // task, it took the job with it silently: the row said "running" until the
            // app was restarted, and the token stayed in `running` for good.
            let job_id = spec.id.clone();
            let runner = Arc::clone(&queue);
            let outcome = queue
                .runtime
                .spawn(async move { runner.run(spec, provider_record, cancel).await })
                .await;
            if let Err(error) = outcome {
                let message = if error.is_panic() {
                    "the job stopped on an internal error - this is a bug in Geis"
                } else {
                    "the job was stopped before it finished"
                };
                queue.fail(&job_id, message);
            }
            drop(permit);
        });

        Ok(record)
    }

    async fn run(
        &self,
        spec: JobSpec,
        provider_record: super::provider::ProviderRecord,
        cancel: CancelToken,
    ) {
        let job_id = spec.id.clone();

        // Cancelled while it waited for a slot. Checked before the log is opened so a job
        // that never ran leaves nothing behind but its row.
        if cancel.is_cancelled() {
            self.running.lock().unwrap().remove(&job_id);
            self.settle(&job_id, JobState::Cancelled, None, None, None);
            return;
        }

        let sink = match LogSink::create(&job_id, &spec.log_path(), Some(self.log_tx.clone())) {
            Ok(sink) => sink,
            Err(error) => {
                self.fail(&job_id, &format!("could not open the job log: {error}"));
                return;
            }
        };

        self.transition(&job_id, JobState::Running);
        sink.note(&format!(
            "{} on {} ({})",
            spec.model_name,
            provider_record.name,
            provider_record.config.kind().as_str()
        ));

        if let Some(probe) = &provider_record.probe {
            for warning in provider::preflight(&spec, probe) {
                sink.error(&warning);
            }
        }

        let provider = provider::build(&provider_record);
        let outcome = provider.execute(&spec, &sink, &cancel).await;
        self.running.lock().unwrap().remove(&job_id);

        match outcome {
            Ok(outcome) if outcome.exit_code == 0 => {
                let artifacts = provider::collect_artifacts(&spec);
                if artifacts.is_empty() && !spec.outputs.is_empty() {
                    // The command succeeded but produced nothing the manifest promised.
                    // Reporting success here would put an empty result in front of the
                    // user with no hint that anything went wrong.
                    sink.error(
                        "no declared outputs were produced - check the manifest's output patterns",
                    );
                    self.settle(
                        &job_id,
                        JobState::Failed,
                        Some(0),
                        Some("finished without producing any declared output"),
                        outcome.external_id.as_deref(),
                    );
                    return;
                }
                sink.note(&format!("done - {} artifact(s)", artifacts.len()));
                let _ = self.store.replace_artifacts(&job_id, &artifacts);
                self.settle(
                    &job_id,
                    JobState::Succeeded,
                    Some(0),
                    None,
                    outcome.external_id.as_deref(),
                );
            }
            Ok(outcome) => {
                // A non-zero exit still leaves partial output worth keeping.
                let artifacts = provider::collect_artifacts(&spec);
                let _ = self.store.replace_artifacts(&job_id, &artifacts);
                self.settle(
                    &job_id,
                    JobState::Failed,
                    Some(outcome.exit_code),
                    Some(&describe_exit(outcome.exit_code)),
                    outcome.external_id.as_deref(),
                );
            }
            Err(ProviderError::Cancelled) => {
                sink.note("cancelled");
                self.settle(&job_id, JobState::Cancelled, None, None, None);
            }
            Err(error) => {
                let message = error.to_string();
                sink.error(&message);
                self.settle(&job_id, JobState::Failed, None, Some(&message), None);
            }
        }
    }

    /// Ask a job to stop, whether it is running or still waiting for a slot. Returns false
    /// if it is neither - already finished, or unknown.
    pub fn cancel(&self, job_id: &str) -> bool {
        let token = self.running.lock().unwrap().get(job_id).cloned();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn running_ids(&self) -> Vec<String> {
        self.running.lock().unwrap().keys().cloned().collect()
    }

    /// Probe a provider and persist what came back, so the settings screen has an
    /// answer immediately next time it opens.
    pub async fn probe_provider(&self, provider_id: &str) -> Result<HostProbe, ProviderError> {
        let record = self
            .store
            .provider(provider_id)
            .map_err(|error| ProviderError::Setup(error.to_string()))?;
        let provider = provider::build(&record);
        let result = provider.probe().await;

        let status = match &result {
            Ok(probe) => {
                let mut warnings = Vec::new();
                if record.config.kind() != super::provider::ProviderKind::Http
                    && !probe.has_tool("python3")
                    && !probe.has_tool("python")
                    && !probe.has_tool("docker")
                {
                    warnings.push(
                        "no python or docker found - most models will have nothing to run in"
                            .to_string(),
                    );
                }
                if warnings.is_empty() {
                    ProviderStatus::Ready {
                        summary: probe.summary(),
                    }
                } else {
                    ProviderStatus::Degraded {
                        summary: probe.summary(),
                        warnings,
                    }
                }
            }
            Err(error) => ProviderStatus::Unreachable {
                error: error.to_string(),
            },
        };
        let _ = self
            .store
            .set_provider_probe(provider_id, result.as_ref().ok(), &status);
        self.events.provider_changed(provider_id);
        result
    }

    fn transition(&self, job_id: &str, state: JobState) {
        let _ = self.store.set_job_state(job_id, state);
        self.emit(job_id);
    }

    fn settle(
        &self,
        job_id: &str,
        state: JobState,
        exit_code: Option<i32>,
        error: Option<&str>,
        external_id: Option<&str>,
    ) {
        let _ = self
            .store
            .finish_job(job_id, state, exit_code, error, external_id);
        self.emit(job_id);
    }

    fn fail(&self, job_id: &str, message: &str) {
        self.running.lock().unwrap().remove(job_id);
        self.settle(job_id, JobState::Failed, None, Some(message), None);
    }

    fn emit(&self, job_id: &str) {
        if let Ok(record) = self.store.job(job_id) {
            self.events.job_changed(&record);
        }
    }
}

/// What a non-zero exit means, where the shell's own conventions say.
///
/// 127 and 126 are the ones a first run meets - the model is not installed where the job
/// was sent - and "exited with code 127" tells nobody that.
fn describe_exit(code: i32) -> String {
    match code {
        127 => "the command was not found on that machine - the model or tool is not installed there, or not on its PATH (exit code 127)".to_string(),
        126 => "the command was found but could not be run - check that it is executable (exit code 126)".to_string(),
        137 => "the job was killed, most often for running out of memory (exit code 137)".to_string(),
        code => format!("exited with code {code}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_command_is_said_in_words() {
        assert!(describe_exit(127).contains("not installed"));
        assert!(describe_exit(137).contains("memory"));
        assert_eq!(describe_exit(2), "exited with code 2");
    }
    use crate::compute::provider::{HttpConfig, ProviderConfig, ProviderRecord};
    use crate::models::manifest::Requirements;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Recorder {
        changes: Mutex<Vec<(String, JobState)>>,
    }

    impl Events for Recorder {
        fn job_changed(&self, record: &JobRecord) {
            self.changes
                .lock()
                .unwrap()
                .push((record.id.clone(), record.state));
        }
        fn job_log(&self, _chunk: LogChunk) {}
        fn provider_changed(&self, _provider_id: &str) {}
    }

    /// An HTTP provider pointed nowhere. A job with no `[http]` section fails on it before
    /// any request is made, so it exercises the queue without a network or a keychain.
    fn store_with_provider(enabled: bool) -> Arc<Store> {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .upsert_provider(&ProviderRecord {
                id: "svc".into(),
                name: "Service".into(),
                config: ProviderConfig::Http(HttpConfig {
                    endpoint: "http://127.0.0.1:9".into(),
                    auth_header: None,
                    auth_format: None,
                    secret_ref: None,
                    headers: BTreeMap::new(),
                    max_parallel: 1,
                }),
                enabled,
                created_at: crate::util::now_iso(),
                probe: None,
                status: ProviderStatus::Unknown,
            })
            .unwrap();
        store
    }

    fn spec(dir: &std::path::Path, id: &str) -> JobSpec {
        JobSpec {
            id: id.into(),
            name: "run".into(),
            model_id: "m".into(),
            model_name: "M".into(),
            task: crate::models::Task::Custom,
            provider_id: "svc".into(),
            world_id: None,
            local_dir: dir.join(id),
            steps: vec![],
            scalars: BTreeMap::new(),
            files: BTreeMap::new(),
            outputs: BTreeMap::new(),
            requirements: Requirements::default(),
            http: None,
        }
    }

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("geis-queue-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn settled(store: &Store, id: &str) -> JobRecord {
        for _ in 0..200 {
            let record = store.job(id).unwrap();
            if record.state.is_terminal() {
                return record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("job {id} never settled");
    }

    #[tokio::test]
    async fn a_provider_that_fails_settles_the_job_as_failed() {
        let dir = scratch();
        let store = store_with_provider(true);
        let events = Arc::new(Recorder::default());
        let queue = Queue::new(store.clone(), events.clone(), Handle::current());
        let record = queue.submit(spec(&dir, "job-a")).unwrap();
        assert_eq!(record.state, JobState::Queued);
        let done = settled(&store, "job-a").await;
        assert_eq!(done.state, JobState::Failed);
        assert!(done.error.unwrap().contains("[http]"));
        assert!(
            queue.running_ids().is_empty(),
            "a finished job holds no token"
        );
        assert!(!queue.cancel("job-a"), "nothing left to cancel");
        let states: Vec<JobState> = events
            .changes
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| *s)
            .collect();
        assert_eq!(states.first(), Some(&JobState::Queued));
        assert_eq!(states.last(), Some(&JobState::Failed));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_job_cancelled_while_waiting_never_starts() {
        let dir = scratch();
        let store = store_with_provider(true);
        let queue = Queue::new(
            store.clone(),
            Arc::new(Recorder::default()),
            Handle::current(),
        );
        // The provider's only slot is taken, so the job waits for it.
        let slot = queue.permit_for("svc", 1).acquire_owned().await.unwrap();
        queue.submit(spec(&dir, "job-b")).unwrap();
        tokio::task::yield_now().await;
        assert_eq!(store.job("job-b").unwrap().state, JobState::Queued);
        assert!(queue.cancel("job-b"), "a waiting job can be cancelled");
        drop(slot);
        let done = settled(&store, "job-b").await;
        assert_eq!(done.state, JobState::Cancelled);
        assert!(
            !dir.join("job-b").join("job.log").exists(),
            "it never opened a log"
        );
        assert!(queue.running_ids().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn submitting_to_a_missing_or_disabled_provider_is_refused() {
        let dir = scratch();
        let store = store_with_provider(false);
        let queue = Queue::new(
            store.clone(),
            Arc::new(Recorder::default()),
            Handle::current(),
        );
        assert!(matches!(
            queue.submit(spec(&dir, "job-c")),
            Err(QueueError::Disabled(_))
        ));
        let mut orphan = spec(&dir, "job-d");
        orphan.provider_id = "gone".into();
        assert!(matches!(queue.submit(orphan), Err(QueueError::Store(_))));
        // Neither left a row behind.
        assert!(store.jobs(None, 10).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_same_job_id_cannot_be_submitted_twice() {
        let dir = scratch();
        let store = store_with_provider(true);
        let queue = Queue::new(
            store.clone(),
            Arc::new(Recorder::default()),
            Handle::current(),
        );
        queue.submit(spec(&dir, "job-e")).unwrap();
        assert!(matches!(
            queue.submit(spec(&dir, "job-e")),
            Err(QueueError::Store(_))
        ));
        settled(&store, "job-e").await;
        std::fs::remove_dir_all(&dir).ok();
    }
}
