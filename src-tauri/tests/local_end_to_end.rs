//! End-to-end exercise of the compute layer with no windowing system involved:
//! a manifest on disk -> registry -> plan -> queue -> pty execution -> artifacts.
//!
//! These are the tests that would catch the failures that actually matter - a job that
//! reports success with no output, a cancel button that leaves the process running, a
//! remote path that was never rewritten.

use geis_lib::compute::exec::LogChunk;
use geis_lib::compute::job::{JobRequest, JobState};
use geis_lib::compute::provider::{
    ExecOptions, LocalConfig, ProviderConfig, ProviderRecord, ProviderStatus,
};
use geis_lib::compute::queue::{Events, Queue};
use geis_lib::models::Registry;
use geis_lib::store::{JobRecord, Store};
use geis_lib::util;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Collects what the queue reports so assertions can be made about it.
#[derive(Default)]
struct Recorder {
    jobs: Mutex<Vec<JobRecord>>,
    log: Mutex<String>,
}

impl Events for Recorder {
    fn job_changed(&self, record: &JobRecord) {
        self.jobs.lock().unwrap().push(record.clone());
    }

    fn job_log(&self, chunk: LogChunk) {
        self.log.lock().unwrap().push_str(&chunk.text);
    }

    fn provider_changed(&self, _provider_id: &str) {}
}

struct Harness {
    root: PathBuf,
    store: Arc<Store>,
    queue: Arc<Queue>,
    registry: Registry,
    recorder: Arc<Recorder>,
}

impl Harness {
    fn new(manifest: &str) -> Self {
        let root = std::env::temp_dir().join(format!("geis-test-{}", util::new_id()));
        let models_dir = root.join("models");
        let jobs_dir = root.join("jobs");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::create_dir_all(&jobs_dir).unwrap();
        std::fs::write(models_dir.join("test-model.toml"), manifest).unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        store
            .upsert_provider(&ProviderRecord {
                id: "local".into(),
                name: "test machine".into(),
                config: ProviderConfig::Local(LocalConfig {
                    exec: ExecOptions {
                        max_parallel: 2,
                        weights_dir: Some(root.join("weights").to_string_lossy().into()),
                        ..Default::default()
                    },
                }),
                enabled: true,
                created_at: util::now_iso(),
                probe: None,
                status: ProviderStatus::Unknown,
            })
            .unwrap();

        let recorder = Arc::new(Recorder::default());
        let queue = Queue::new(
            Arc::clone(&store),
            Arc::clone(&recorder) as Arc<dyn Events>,
            tokio::runtime::Handle::current(),
        );

        Self {
            registry: Registry::load_dirs(&[models_dir]),
            root,
            store,
            queue,
            recorder,
        }
    }

    fn jobs_dir(&self) -> PathBuf {
        self.root.join("jobs")
    }

    fn submit(&self, request: JobRequest) -> JobRecord {
        let manifest = self.registry.get(&request.model_id).expect("model loaded");
        let spec = geis_lib::compute::job::plan(manifest, &request, &self.jobs_dir())
            .expect("plan succeeds");
        self.queue.submit(spec).expect("submit succeeds")
    }

    /// Polls the database rather than the event stream: the database is what the UI reads
    /// after a restart, so it is the thing that has to be right.
    async fn wait_for_terminal(&self, job_id: &str) -> JobRecord {
        for _ in 0..300 {
            let record = self.store.job(job_id).unwrap();
            if record.state.is_terminal() {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("job {job_id} never reached a terminal state");
    }

    fn log(&self) -> String {
        self.recorder.log.lock().unwrap().clone()
    }

    fn states_seen(&self) -> Vec<JobState> {
        self.recorder
            .jobs
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.state)
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const ECHO_MODEL: &str = r#"
schema = 1
id = "test-echo"
name = "Test Echo"
task = "structure_prediction"

[requirements]
gpu = "none"
runtime = "binary"
weights = "none"

[inputs]
sequence = { type = "fasta", required = true }
label = { type = "text", default = "hello" }
copies = { type = "int", default = 2, min = 1, max = 5 }

[outputs]
structures = { type = "structure", pattern = "out/*.cif", primary = true }

[[run]]
label = "write a structure"
command = """
mkdir -p {{job.workdir}}/out
echo MARKER {{input.label}}
head -1 {{input.sequence.path}}
for i in $(seq 1 {{input.copies}}); do
  echo data_model_$i > {{job.workdir}}/out/model_$i.cif
done
"""
"#;

#[tokio::test]
async fn runs_a_job_and_collects_what_the_manifest_declared() {
    let harness = Harness::new(ECHO_MODEL);
    let record = harness.submit(JobRequest {
        model_id: "test-echo".into(),
        provider_id: "local".into(),
        world_id: Some("world-1".into()),
        name: Some("echo run".into()),
        inputs: [
            ("sequence".to_string(), serde_json::json!("mkt aylk")),
            ("label".to_string(), serde_json::json!("from test")),
            ("copies".to_string(), serde_json::json!(3)),
        ]
        .into_iter()
        .collect(),
    });

    let finished = harness.wait_for_terminal(&record.id).await;
    assert_eq!(
        finished.state,
        JobState::Succeeded,
        "log:\n{}",
        harness.log()
    );
    assert_eq!(finished.exit_code, Some(0));
    assert_eq!(finished.world_id.as_deref(), Some("world-1"));

    // Declared outputs became artifacts, and the primary one sorts first.
    let artifacts = harness.store.artifacts(&record.id).unwrap();
    assert_eq!(artifacts.len(), 3, "expected one artifact per copy");
    assert!(artifacts.iter().all(|artifact| artifact.primary));
    assert!(artifacts.iter().all(|artifact| artifact.path.exists()));
    assert!(artifacts.iter().all(|artifact| artifact.bytes > 0));

    // Scalar and file inputs both reached the command.
    let log = harness.log();
    assert!(log.contains("MARKER from test"), "log:\n{log}");
    assert!(
        log.contains(">query"),
        "the staged FASTA was not read:\n{log}"
    );

    // The log file survives independently of the event stream.
    let on_disk = std::fs::read_to_string(finished.spec.log_path()).unwrap();
    assert!(on_disk.contains("MARKER from test"));

    assert!(harness.states_seen().contains(&JobState::Running));
}

const NO_OUTPUT_MODEL: &str = r#"
schema = 1
id = "test-silent"
name = "Test Silent"
task = "custom"

[requirements]
gpu = "none"
runtime = "binary"
weights = "none"

[outputs]
structures = { type = "structure", pattern = "out/*.cif", primary = true }

[[run]]
command = "echo 'I did nothing useful'"
"#;

#[tokio::test]
async fn a_command_that_produces_nothing_is_not_reported_as_success() {
    // The failure mode this guards against: exit code 0, no output, and a user staring at
    // an empty result wondering where their structure went.
    let harness = Harness::new(NO_OUTPUT_MODEL);
    let record = harness.submit(JobRequest {
        model_id: "test-silent".into(),
        provider_id: "local".into(),
        world_id: None,
        name: None,
        inputs: Default::default(),
    });

    let finished = harness.wait_for_terminal(&record.id).await;
    assert_eq!(finished.state, JobState::Failed);
    assert!(
        finished
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("without producing any declared output"),
        "error was {:?}",
        finished.error
    );
}

const FAILING_MODEL: &str = r#"
schema = 1
id = "test-fails"
name = "Test Fails"
task = "custom"

[requirements]
gpu = "none"
runtime = "binary"
weights = "none"

[[run]]
label = "first"
command = "echo step-one-ran"

[[run]]
label = "second"
command = "echo about-to-fail; exit 17"

[[run]]
label = "third"
command = "echo step-three-should-not-run"
"#;

#[tokio::test]
async fn a_failing_step_stops_the_job_and_keeps_the_exit_code() {
    let harness = Harness::new(FAILING_MODEL);
    let record = harness.submit(JobRequest {
        model_id: "test-fails".into(),
        provider_id: "local".into(),
        world_id: None,
        name: None,
        inputs: Default::default(),
    });

    let finished = harness.wait_for_terminal(&record.id).await;
    assert_eq!(finished.state, JobState::Failed);
    assert_eq!(finished.exit_code, Some(17));

    let log = harness.log();
    assert!(log.contains("step-one-ran"));
    assert!(
        !log.contains("step-three-should-not-run"),
        "steps after a failure must not run:\n{log}"
    );
}

const SLOW_MODEL: &str = r#"
schema = 1
id = "test-slow"
name = "Test Slow"
task = "custom"

[requirements]
gpu = "none"
runtime = "binary"
weights = "none"

[[run]]
command = "echo started; sleep 120; touch {{job.workdir}}/completed.flag"
"#;

#[tokio::test]
async fn cancelling_stops_the_process() {
    let harness = Harness::new(SLOW_MODEL);
    let record = harness.submit(JobRequest {
        model_id: "test-slow".into(),
        provider_id: "local".into(),
        world_id: None,
        name: None,
        inputs: Default::default(),
    });

    // Wait until the child is actually running before cancelling; cancelling a queued job
    // takes a different path.
    for _ in 0..100 {
        if harness.log().contains("started") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        harness.queue.cancel(&record.id),
        "job should have been running"
    );

    // Timing is the assertion that matters: if the child were merely orphaned rather than
    // killed, this would sit here for the full two minutes. The log cannot be used for
    // this - the command line itself is echoed into it, so any marker word appears there
    // whether or not it ever ran.
    let started = std::time::Instant::now();
    let finished = harness.wait_for_terminal(&record.id).await;
    assert_eq!(finished.state, JobState::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "cancelling took {:?} - the process was not actually killed",
        started.elapsed()
    );
    assert!(
        !finished.local_dir.join("completed.flag").exists(),
        "the command ran to completion despite being cancelled"
    );
}

#[tokio::test]
async fn a_missing_required_input_is_refused_before_anything_runs() {
    let harness = Harness::new(ECHO_MODEL);
    let manifest = harness.registry.get("test-echo").unwrap();
    let error = geis_lib::compute::job::plan(
        manifest,
        &JobRequest {
            model_id: "test-echo".into(),
            provider_id: "local".into(),
            world_id: None,
            name: None,
            inputs: Default::default(),
        },
        &harness.jobs_dir(),
    )
    .expect_err("a missing sequence must not plan");
    assert!(error.to_string().contains("sequence"), "{error}");
}

#[tokio::test]
async fn out_of_range_values_are_refused_with_the_bound_that_was_broken() {
    let harness = Harness::new(ECHO_MODEL);
    let manifest = harness.registry.get("test-echo").unwrap();
    let error = geis_lib::compute::job::plan(
        manifest,
        &JobRequest {
            model_id: "test-echo".into(),
            provider_id: "local".into(),
            world_id: None,
            name: None,
            inputs: [
                ("sequence".to_string(), serde_json::json!("MKT")),
                ("copies".to_string(), serde_json::json!(99)),
            ]
            .into_iter()
            .collect(),
        },
        &harness.jobs_dir(),
    )
    .expect_err("99 copies is outside the declared range");
    assert!(error.to_string().contains("<= 5"), "{error}");
}

#[tokio::test]
async fn the_local_probe_describes_this_machine() {
    let harness = Harness::new(ECHO_MODEL);
    let probe = harness
        .queue
        .probe_provider("local")
        .await
        .expect("probing this machine should work");
    assert!(probe.complete);
    assert!(probe.cores > 0);
    assert!(!probe.hostname.is_empty());
    assert!(
        probe.has_tool("sh") || probe.has_tool("python3") || probe.has_tool("git"),
        "the probe found no tooling at all: {:?}",
        probe.tools
    );

    // The status is persisted, so the settings screen has an answer without re-probing.
    let stored = harness.store.provider("local").unwrap();
    assert!(stored.probe.is_some());
    assert!(matches!(
        stored.status,
        ProviderStatus::Ready { .. } | ProviderStatus::Degraded { .. }
    ));
}
