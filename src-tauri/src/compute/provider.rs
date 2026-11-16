//! Where a job can run.
//!
//! A provider is one configured place to execute work: this machine, a workstation over
//! ssh, a Slurm cluster, or an HTTP endpoint. Providers receive a fully planned `JobSpec`
//! and never look at manifests - that separation is what lets a new model be a TOML file
//! and a new kind of compute be one impl of this trait, with neither knowing about the
//! other.

use super::exec::{CancelToken, LogSink};
use super::job::JobSpec;
use super::probe::HostProbe;
use crate::models::template::shell_quote;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Local,
    Ssh,
    Slurm,
    Http,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Local => "local",
            ProviderKind::Ssh => "ssh",
            ProviderKind::Slurm => "slurm",
            ProviderKind::Http => "http",
        }
    }
}

/// Options common to anywhere a shell command runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecOptions {
    /// Directory jobs get their workdir under, on the executing host.
    pub scratch: Option<String>,
    /// Root for model weights on the executing host. Exposed as `{{provider.weights_dir}}`
    /// so a manifest never hardcodes a path - the same manifest works on a laptop and a
    /// cluster with a shared weights mount.
    pub weights_dir: Option<String>,
    /// Extra environment for every step. Where CUDA_VISIBLE_DEVICES, HF_HOME, and licence
    /// variables live.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Lines run before the first step - `conda activate`, `module load cuda/12.4`.
    #[serde(default)]
    pub prelude: Vec<String>,
    /// Jobs this provider will run at once.
    #[serde(default = "one")]
    pub max_parallel: usize,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(flatten)]
    pub exec: ExecOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    /// An ssh destination: `user@host`, or better, a `~/.ssh/config` alias.
    ///
    /// We shell out to the system `ssh` rather than linking an SSH client so that jump
    /// hosts, ProxyCommand, agent forwarding, hardware keys, and kerberos all keep
    /// working. Clusters use every one of those, and an embedded client fights them all.
    pub host: String,
    /// Absolute path on the remote host that job workdirs are created under.
    pub remote_dir: String,
    /// Extra `ssh` flags, e.g. `["-p", "2222"]`.
    #[serde(default)]
    pub ssh_options: Vec<String>,
    #[serde(flatten)]
    pub exec: ExecOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlurmConfig {
    #[serde(flatten)]
    pub ssh: SshConfig,
    pub partition: Option<String>,
    pub account: Option<String>,
    /// Raw `--gres`/`--gpus` value, e.g. `a100:1`. Sites differ too much to model this.
    pub gpus: Option<String>,
    pub cpus_per_task: Option<u32>,
    /// Slurm memory syntax, e.g. `64G`.
    pub mem: Option<String>,
    /// Slurm time syntax, e.g. `12:00:00`.
    pub time_limit: Option<String>,
    /// Extra `#SBATCH` lines, verbatim, for site-specific requirements.
    #[serde(default)]
    pub extra_sbatch: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Base URL, available to manifests as `{{provider.endpoint}}`.
    pub endpoint: String,
    /// Header the credential goes in, e.g. `Authorization` or `x-api-key`.
    pub auth_header: Option<String>,
    /// Format string for the credential, e.g. `Bearer {}`.
    pub auth_format: Option<String>,
    /// Keychain entry holding the credential. The secret itself is never written to the
    /// database, the config, or a log.
    pub secret_ref: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "one")]
    pub max_parallel: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderConfig {
    Local(LocalConfig),
    Ssh(SshConfig),
    Slurm(SlurmConfig),
    Http(HttpConfig),
}

/// An optional field the form sent back empty is a field nobody filled in.
fn blank_to_none(value: &mut Option<String>) {
    if let Some(text) = value {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != text.len() {
            *value = Some(trimmed.to_string());
        }
    }
}

impl ExecOptions {
    fn tidy(&mut self) {
        blank_to_none(&mut self.scratch);
        blank_to_none(&mut self.weights_dir);
        self.prelude.retain(|line| !line.trim().is_empty());
        self.env.retain(|key, _| !key.trim().is_empty());
        self.max_parallel = self.max_parallel.max(1);
    }
}

impl SshConfig {
    fn tidy(&mut self) -> Result<(), String> {
        self.host = self.host.trim().to_string();
        self.remote_dir = self.remote_dir.trim().to_string();
        if self.host.is_empty() {
            return Err("give the host to connect to".into());
        }
        // A leading dash would be read by ssh as an option rather than a destination.
        if self.host.starts_with('-') {
            return Err("the host cannot start with a dash".into());
        }
        // `~` is expanded by some of the commands that see this path and quoted by the
        // rest, so a home-relative directory ends up as two different places.
        if !self.remote_dir.starts_with('/') {
            return Err(
                "the remote directory must be an absolute path, such as /scratch/you/geis".into(),
            );
        }
        // The host goes to ssh as one argument and into rsync's `host:path`; a space or a
        // line break in it is never a host and would split one.
        if self
            .host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("the host cannot contain spaces or line breaks".into());
        }
        // The remote directory reaches the far side unquoted in two places that cannot be
        // quoted for it: rsync's `host:path` (an rsync as old as macOS ships hands the path
        // to the remote shell, which splits it at a space) and sbatch's `--output=`, which
        // ends at a space or a `#`. A directory that needs no quoting works everywhere.
        if shell_quote(&self.remote_dir) != self.remote_dir {
            return Err("the remote directory can only use letters, digits and . _ - / = : @ , + - no spaces".into());
        }
        if self
            .ssh_options
            .iter()
            .any(|option| option.chars().any(char::is_control))
        {
            return Err("an ssh option cannot contain a line break".into());
        }
        self.ssh_options.retain(|option| !option.trim().is_empty());
        self.exec.tidy();
        Ok(())
    }
}

impl ProviderConfig {
    /// A draft from the settings form, made safe to store.
    ///
    /// A cleared text field arrives as an empty string, and an empty string is not
    /// nothing: `--partition=` is rejected by sbatch, and an empty weights directory
    /// counts as "weights are set" and unblocks a model that cannot run.
    pub fn tidied(mut self) -> Result<Self, String> {
        match &mut self {
            ProviderConfig::Local(config) => config.exec.tidy(),
            ProviderConfig::Ssh(config) => config.tidy()?,
            ProviderConfig::Slurm(config) => {
                config.ssh.tidy()?;
                blank_to_none(&mut config.partition);
                blank_to_none(&mut config.account);
                blank_to_none(&mut config.gpus);
                blank_to_none(&mut config.mem);
                blank_to_none(&mut config.time_limit);
                config.extra_sbatch.retain(|line| !line.trim().is_empty());
                // Each of these is written onto one `#SBATCH` line. A line break in one
                // ends the directives there and turns the rest into a command the batch
                // script runs on the cluster.
                let one_line = [
                    &config.partition,
                    &config.account,
                    &config.gpus,
                    &config.mem,
                    &config.time_limit,
                ]
                .into_iter()
                .flatten()
                .chain(config.extra_sbatch.iter())
                .all(|value| !value.chars().any(char::is_control));
                if !one_line {
                    return Err("a Slurm setting cannot contain a line break".into());
                }
            }
            ProviderConfig::Http(config) => {
                config.endpoint = config.endpoint.trim().trim_end_matches('/').to_string();
                if !config.endpoint.starts_with("http://")
                    && !config.endpoint.starts_with("https://")
                {
                    return Err("the endpoint must be an http:// or https:// URL".into());
                }
                blank_to_none(&mut config.auth_header);
                blank_to_none(&mut config.auth_format);
                blank_to_none(&mut config.secret_ref);
                config.max_parallel = config.max_parallel.max(1);
            }
        }
        Ok(self)
    }

    pub fn kind(&self) -> ProviderKind {
        match self {
            ProviderConfig::Local(_) => ProviderKind::Local,
            ProviderConfig::Ssh(_) => ProviderKind::Ssh,
            ProviderConfig::Slurm(_) => ProviderKind::Slurm,
            ProviderConfig::Http(_) => ProviderKind::Http,
        }
    }

    pub fn max_parallel(&self) -> usize {
        let value = match self {
            ProviderConfig::Local(c) => c.exec.max_parallel,
            ProviderConfig::Ssh(c) => c.exec.max_parallel,
            ProviderConfig::Slurm(c) => c.ssh.exec.max_parallel,
            ProviderConfig::Http(c) => c.max_parallel,
        };
        value.max(1)
    }

    pub fn exec_options(&self) -> Option<&ExecOptions> {
        match self {
            ProviderConfig::Local(c) => Some(&c.exec),
            ProviderConfig::Ssh(c) => Some(&c.exec),
            ProviderConfig::Slurm(c) => Some(&c.ssh.exec),
            ProviderConfig::Http(_) => None,
        }
    }
}

/// A provider row as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub config: ProviderConfig,
    pub enabled: bool,
    pub created_at: String,
    /// Result of the last probe, cached so the UI has something to show immediately.
    pub probe: Option<HostProbe>,
    pub status: ProviderStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Never probed.
    #[default]
    Unknown,
    Ready {
        summary: String,
    },
    /// Reachable, but something a job will need is missing.
    Degraded {
        summary: String,
        warnings: Vec<String>,
    },
    Unreachable {
        error: String,
    },
}

/// Cancellation has to survive the trip up from `exec`: a `?` that wrapped it in
/// `Exec(Cancelled)` would reach the queue as a generic failure, and a job the user
/// stopped on purpose would be reported as broken.
impl From<super::exec::ExecError> for ProviderError {
    fn from(error: super::exec::ExecError) -> Self {
        match error {
            super::exec::ExecError::Cancelled => ProviderError::Cancelled,
            other => ProviderError::Exec(other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    Setup(String),
    #[error("{0}")]
    Exec(super::exec::ExecError),
    #[error("{0}")]
    Remote(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("template: {0}")]
    Template(#[from] crate::models::template::TemplateError),
    #[error("cancelled")]
    Cancelled,
}

/// Result of one execution attempt.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub exit_code: i32,
    /// Scheduler or service id, when there is one worth showing (`slurm 4821`).
    pub external_id: Option<String>,
}

/// A file a job produced, matched against the manifest's declared outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub kind: crate::models::OutputKind,
    pub path: PathBuf,
    pub bytes: u64,
    pub primary: bool,
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> ProviderKind;

    /// Values exposed to manifests as `{{provider.*}}`.
    fn vars(&self) -> BTreeMap<String, String>;

    /// Ask the host what it is and what it has. Cheap enough to run on demand.
    async fn probe(&self) -> Result<HostProbe, ProviderError>;

    /// Stage inputs, run every step, and bring results back into `spec.local_dir`.
    async fn execute(
        &self,
        spec: &JobSpec,
        sink: &LogSink,
        cancel: &CancelToken,
    ) -> Result<RunOutcome, ProviderError>;
}

/// Turns a persisted config into something runnable.
pub fn build(record: &ProviderRecord) -> Box<dyn Provider> {
    match &record.config {
        ProviderConfig::Local(config) => Box::new(super::local::LocalProvider::new(
            record.id.clone(),
            config.clone(),
        )),
        ProviderConfig::Ssh(config) => Box::new(super::ssh::SshProvider::new(
            record.id.clone(),
            config.clone(),
        )),
        ProviderConfig::Slurm(config) => Box::new(super::slurm::SlurmProvider::new(
            record.id.clone(),
            config.clone(),
        )),
        ProviderConfig::Http(config) => Box::new(super::http::HttpProvider::new(
            record.id.clone(),
            config.clone(),
        )),
    }
}

/// Turn a completed job's workdir into the artifact list its manifest declared.
///
/// Shared by every provider: by the time this runs, results are on local disk regardless
/// of where they were computed.
pub fn collect_artifacts(spec: &JobSpec) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    // The workdir is a path, not a pattern: a `[` or `*` in it - in a user name, or a
    // data directory someone moved - would otherwise be read as glob syntax and match
    // nothing, and a finished job would report that it produced no output.
    let Some(base) = spec.local_dir.to_str() else {
        return artifacts;
    };
    let base = glob::Pattern::escape(base);
    for (name, output) in &spec.outputs {
        // Manifests are checked for this when they load, but a job planned by an older
        // build carries its patterns in the database.
        let escapes = std::path::Path::new(&output.pattern)
            .components()
            .any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            });
        if escapes {
            continue;
        }
        let pattern = format!("{}/{}", base.trim_end_matches('/'), output.pattern);
        let Ok(paths) = glob::glob(&pattern) else {
            continue;
        };
        for path in paths.filter_map(Result::ok) {
            let bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
            if path.is_file() {
                artifacts.push(Artifact {
                    name: name.clone(),
                    kind: output.kind,
                    path,
                    bytes,
                    primary: output.primary,
                });
            }
        }
    }
    artifacts.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.path.cmp(&b.path)));
    artifacts
}

/// Warnings a job would hit on this host, checked before submitting rather than 20
/// minutes in. Advisory: the user can always run anyway, because probes are imperfect and
/// a login node legitimately looks nothing like the compute node.
pub fn preflight(spec: &JobSpec, probe: &HostProbe) -> Vec<String> {
    use crate::models::GpuNeed;
    let mut warnings = Vec::new();

    match spec.requirements.gpu {
        GpuNeed::Required if !probe.has_gpu() => {
            warnings.push(format!(
                "{} needs a GPU and none was detected on {}",
                spec.model_name, probe.hostname
            ));
        }
        GpuNeed::Recommended if !probe.has_gpu() => {
            warnings.push(format!(
                "no GPU detected on {} - this will be slow",
                probe.hostname
            ));
        }
        _ => {}
    }

    if let (Some(needed), Some(available)) = (spec.requirements.vram_gb, probe.max_vram_gb()) {
        if available < needed {
            warnings.push(format!(
                "{} wants {needed} GiB of GPU memory, largest device has {available} GiB",
                spec.model_name
            ));
        }
    }

    for binary in &spec.requirements.needs_binaries {
        if !probe.has_tool(binary) {
            warnings.push(format!(
                "{binary:?} was not found on PATH on {}",
                probe.hostname
            ));
        }
    }

    if let (Some(needed), Some(free)) = (spec.requirements.disk_gb, probe.scratch_free_gb) {
        if free < needed {
            warnings.push(format!(
                "scratch has {free} GiB free, job wants {needed} GiB"
            ));
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::manifest::{OutputKind, OutputSpec, Requirements};

    fn ssh(host: &str, remote_dir: &str) -> Result<ProviderConfig, String> {
        let config: ProviderConfig = serde_json::from_value(serde_json::json!({
            "kind": "ssh", "host": host, "remote_dir": remote_dir,
        }))
        .unwrap();
        config.tidied()
    }

    fn slurm(field: &str, value: serde_json::Value) -> Result<ProviderConfig, String> {
        let mut draft = serde_json::json!({
            "kind": "slurm", "host": "login", "remote_dir": "/scratch/me",
        });
        draft[field] = value;
        serde_json::from_value::<ProviderConfig>(draft)
            .unwrap()
            .tidied()
    }

    #[test]
    fn a_host_or_directory_that_would_break_a_command_is_refused() {
        assert!(ssh("gpu-box", "/scratch/me/geis").is_ok());
        assert!(
            ssh("  me@gpu-box  ", " /scratch/me ").is_ok(),
            "surrounding blanks are tidied"
        );
        assert!(ssh("", "/scratch").is_err());
        assert!(ssh("-oProxyCommand=touch /tmp/x", "/scratch").is_err());
        assert!(ssh("gpu box", "/scratch").is_err());
        assert!(ssh("gpu\nbox", "/scratch").is_err());
        assert!(ssh("gpu", "~/geis").is_err());
        assert!(ssh("gpu", "relative/dir").is_err());
        // Each of these reaches rsync's `host:path` or sbatch's `--output=` unquoted.
        for bad in [
            "/scratch/my runs",
            "/scratch/$USER",
            "/scratch/a;b",
            "/scratch/#1",
            "/scratch/a\nb",
            "/scratch/`id`",
        ] {
            assert!(ssh("gpu", bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_slurm_setting_cannot_smuggle_in_a_command() {
        assert!(slurm("partition", "gpu".into()).is_ok());
        assert!(slurm("partition", "gpu\nrm -rf ~".into()).is_err());
        assert!(slurm("account", "lab\r\nid".into()).is_err());
        assert!(slurm(
            "extra_sbatch",
            serde_json::json!(["--qos=high", "--x\ncurl evil"])
        )
        .is_err());
        // A cleared field is no setting at all, not an empty `--partition=`.
        match slurm("partition", "  ".into()).unwrap() {
            ProviderConfig::Slurm(config) => assert!(config.partition.is_none()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn an_http_endpoint_must_be_a_url() {
        let http = |endpoint: &str| {
            serde_json::from_value::<ProviderConfig>(
                serde_json::json!({"kind": "http", "endpoint": endpoint}),
            )
            .unwrap()
            .tidied()
        };
        assert!(http("file:///etc/passwd").is_err());
        assert!(http("api.example").is_err());
        match http(" https://api.example/v1/ ").unwrap() {
            ProviderConfig::Http(config) => assert_eq!(config.endpoint, "https://api.example/v1"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_draft_missing_its_kind_or_fields_is_an_error() {
        assert!(
            serde_json::from_value::<ProviderConfig>(serde_json::json!({"host": "x"})).is_err()
        );
        assert!(
            serde_json::from_value::<ProviderConfig>(serde_json::json!({"kind": "ssh"})).is_err()
        );
        assert!(
            serde_json::from_value::<ProviderConfig>(serde_json::json!({"kind": "teleport"}))
                .is_err()
        );
        // Zero jobs at once would leave every job waiting for ever.
        let local: ProviderConfig =
            serde_json::from_value(serde_json::json!({"kind": "local", "max_parallel": 0}))
                .unwrap();
        assert_eq!(local.tidied().unwrap().max_parallel(), 1);
    }

    fn spec_in(dir: &std::path::Path, outputs: &[(&str, &str)]) -> JobSpec {
        JobSpec {
            id: "id".into(),
            name: "run".into(),
            model_id: "m".into(),
            model_name: "M".into(),
            task: crate::models::Task::Custom,
            provider_id: "local".into(),
            world_id: None,
            local_dir: dir.to_path_buf(),
            steps: vec![],
            scalars: BTreeMap::new(),
            files: BTreeMap::new(),
            outputs: outputs
                .iter()
                .map(|(name, pattern)| {
                    (
                        name.to_string(),
                        OutputSpec {
                            kind: OutputKind::Any,
                            pattern: pattern.to_string(),
                            label: None,
                            primary: *name == "main",
                        },
                    )
                })
                .collect(),
            requirements: Requirements::default(),
            http: None,
        }
    }

    #[test]
    fn artifacts_are_found_in_a_workdir_whose_name_looks_like_a_pattern() {
        let root = std::env::temp_dir().join(format!("geis-artifacts-{}", uuid::Uuid::new_v4()));
        let dir = root.join("run [1] *");
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("out/model.cif"), b"data_x").unwrap();
        std::fs::write(dir.join("scores.csv"), b"a,b").unwrap();
        std::fs::write(root.join("outside.cif"), b"not this job's").unwrap();

        let found = collect_artifacts(&spec_in(
            &dir,
            &[
                ("main", "out/*.cif"),
                ("scores", "*.csv"),
                ("escape", "../*.cif"),
            ],
        ));
        let names: Vec<_> = found
            .iter()
            .map(|a| a.path.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["model.cif", "scores.csv"],
            "primary first, nothing from outside"
        );
        assert!(found[0].primary);
        assert_eq!(found[0].bytes, 6);
        std::fs::remove_dir_all(&root).ok();
    }
}
