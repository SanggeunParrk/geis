//! Submits jobs to a Slurm cluster through an ssh login node.
//!
//! Everything a site can customise - partition, account, gres syntax, module preludes,
//! extra #SBATCH lines - is configuration, because no two clusters agree on any of it.

use super::exec::{CancelToken, LogSink};
use super::job::JobSpec;
use super::probe::{HostProbe, PROBE_SCRIPT};
use super::provider::{Provider, ProviderError, ProviderKind, RunOutcome, SlurmConfig};
use super::remote::{remote_workdir, Remote};
use super::ssh::write_step_scripts;
use crate::models::template::shell_quote;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// How often the login node is asked about the job. Slurm's own accounting refreshes on
/// this order anyway, and hammering a login node from a GUI is how one gets banned.
const POLL: Duration = Duration::from_secs(10);

/// How many polls in a row may fail to reach the login node before the job is given up
/// on: half an hour, which outlasts a closed lid over lunch but not a cluster that is gone.
const MAX_UNREACHABLE_POLLS: u32 = 180;

pub struct SlurmProvider {
    id: String,
    config: SlurmConfig,
}

impl SlurmProvider {
    pub fn new(id: String, config: SlurmConfig) -> Self {
        Self { id, config }
    }

    fn remote(&self) -> Remote {
        Remote::new(
            self.config.ssh.host.clone(),
            self.config.ssh.ssh_options.clone(),
        )
    }

    /// The batch script. Each step is a separate `bash -l` invocation, matching how the
    /// local and ssh providers run them, so a manifest cannot accidentally depend on
    /// state leaking between steps on one provider but not another.
    fn sbatch_script(&self, spec: &JobSpec, workdir: &str, steps: &[String]) -> String {
        let mut script = String::from("#!/bin/bash\n");
        let mut directive = |line: String| script.push_str(&format!("#SBATCH {line}\n"));

        // By characters, not bytes: an id read back from an older database need not be a
        // uuid, and slicing a short one panics inside the job's task.
        let short: String = spec
            .id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(8)
            .collect();
        directive(format!("--job-name=geis-{short}"));
        // stdout and stderr into one file: interleaved output is what the terminal panel
        // shows, and separating them makes a traceback and the line that caused it land
        // in different places.
        directive(format!("--output={workdir}/slurm.out"));
        directive("--open-mode=append".to_string());
        if let Some(partition) = &self.config.partition {
            directive(format!("--partition={partition}"));
        }
        if let Some(account) = &self.config.account {
            directive(format!("--account={account}"));
        }
        if let Some(gpus) = &self.config.gpus {
            directive(format!("--gres=gpu:{gpus}"));
        }
        if let Some(cpus) = self.config.cpus_per_task {
            directive(format!("--cpus-per-task={cpus}"));
        }
        if let Some(mem) = &self.config.mem {
            directive(format!("--mem={mem}"));
        }
        if let Some(time) = &self.config.time_limit {
            directive(format!("--time={time}"));
        }
        for extra in &self.config.extra_sbatch {
            directive(extra.clone());
        }

        script.push('\n');
        for (key, value) in &self.config.ssh.exec.env {
            if !super::script::is_env_name(key) {
                continue;
            }
            script.push_str(&format!("export {key}={}\n", shell_quote(value)));
        }
        script.push_str(&format!("cd {}\n", shell_quote(workdir)));
        script.push_str("status=0\n");
        for relative in steps {
            script.push_str(&format!(
                "if [ $status -eq 0 ]; then bash -l {} || status=$?; fi\n",
                shell_quote(&format!("{workdir}/{relative}"))
            ));
        }
        // The sentinel is how completion is detected. `sacct` is not configured on every
        // cluster and `squeue` forgets finished jobs within minutes, so relying on either
        // alone means a finished job that looks lost.
        script.push_str(&format!(
            "printf '%s' \"$status\" > {}\n",
            shell_quote(&format!("{workdir}/.geis/exit_code"))
        ));
        script.push_str("exit $status\n");
        script
    }
}

#[async_trait::async_trait]
impl Provider for SlurmProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Slurm
    }

    fn vars(&self) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        vars.insert("host".into(), self.config.ssh.host.clone());
        vars.insert("remote_dir".into(), self.config.ssh.remote_dir.clone());
        if let Some(scratch) = &self.config.ssh.exec.scratch {
            vars.insert("scratch".into(), scratch.clone());
        }
        if let Some(weights) = &self.config.ssh.exec.weights_dir {
            vars.insert("weights_dir".into(), weights.clone());
        }
        if let Some(partition) = &self.config.partition {
            vars.insert("partition".into(), partition.clone());
        }
        vars
    }

    async fn probe(&self) -> Result<HostProbe, ProviderError> {
        let captured = self
            .remote()
            .capture_script(PROBE_SCRIPT, Duration::from_secs(90))
            .await?;
        let mut probe = HostProbe::parse(&captured.stdout);
        if !probe.complete {
            return Err(ProviderError::Unreachable(format!(
                "probe of {} did not finish: {}",
                self.config.ssh.host,
                captured.message()
            )));
        }
        // A login node has no GPUs and that is not a problem, so anything discovered here
        // describes the wrong machine. Ask the scheduler what the partition actually has.
        probe.gpus.clear();
        if let Ok(captured) = self
            .remote()
            .run_quiet(
                &sinfo_command(self.config.partition.as_deref()),
                Duration::from_secs(30),
            )
            .await
        {
            for gpu in parse_sinfo_gres(&captured.stdout) {
                probe.gpus.push(gpu);
            }
        }
        Ok(probe)
    }

    async fn execute(
        &self,
        spec: &JobSpec,
        sink: &LogSink,
        cancel: &CancelToken,
    ) -> Result<RunOutcome, ProviderError> {
        let remote = self.remote();
        let workdir = remote_workdir(&self.config.ssh.remote_dir, &spec.local_dir);

        sink.note(&format!("staging to {}:{}", self.config.ssh.host, workdir));
        remote.mkdir(&format!("{workdir}/.geis")).await?;
        let steps =
            write_step_scripts(spec, &workdir, &self.vars(), &self.config.ssh.exec.prelude)?;
        let batch = self.sbatch_script(spec, &workdir, &steps);
        std::fs::write(spec.local_dir.join(".geis/sbatch.sh"), &batch)?;
        remote.upload(&spec.local_dir, &workdir).await?;

        let submit = format!(
            "sbatch --parsable {}",
            shell_quote(&format!("{workdir}/.geis/sbatch.sh"))
        );
        let captured = remote.run_quiet(&submit, Duration::from_secs(120)).await?;
        if !captured.ok() {
            return Err(ProviderError::Remote(format!(
                "sbatch failed: {}",
                captured.message()
            )));
        }
        let slurm_id = parse_sbatch_id(&captured.stdout).ok_or_else(|| {
            ProviderError::Remote(format!(
                "sbatch did not return a job id: {}",
                captured.message()
            ))
        })?;
        sink.note(&format!("submitted as slurm job {slurm_id}"));

        let out_path = format!("{workdir}/slurm.out");
        let exit_path = format!("{workdir}/.geis/exit_code");
        let mut offset: u64 = 0;
        let mut last_state = String::new();
        let mut missing_polls = 0;
        let mut unreachable_polls = 0u32;

        let exit_code = loop {
            if cancel.is_cancelled() {
                sink.note(&format!("cancelling slurm job {slurm_id}"));
                let _ = remote
                    .run_quiet(&format!("scancel {slurm_id}"), Duration::from_secs(30))
                    .await;
                return Err(ProviderError::Cancelled);
            }

            if let Ok((chunk, consumed)) = remote.tail_from(&out_path, offset).await {
                offset += consumed;
                if !chunk.is_empty() {
                    sink.write_bytes(chunk.replace('\n', "\r\n").as_bytes());
                }
            }

            // One round of questions to the login node. A round that cannot be asked -
            // the laptop slept, the wifi dropped, the login node was restarted - says
            // nothing about the job, which is running on a compute node regardless. Only
            // a long run of them is given up on, and even then the job is left alone on
            // the cluster rather than cancelled: its results are still worth fetching by
            // hand.
            let round = async {
                let sentinel = remote
                    .run_quiet(
                        &format!("cat {} 2>/dev/null || true", shell_quote(&exit_path)),
                        Duration::from_secs(30),
                    )
                    .await?;
                if let Ok(code) = sentinel.stdout.trim().parse::<i32>() {
                    return Ok::<_, ProviderError>((Some(code), String::new()));
                }
                let state = remote
                    .run_quiet(
                        &format!("squeue -h -j {slurm_id} -o %T 2>/dev/null || true"),
                        Duration::from_secs(30),
                    )
                    .await?;
                Ok((None, state.stdout.trim().to_string()))
            };
            let state = match round.await {
                Ok((Some(code), _)) => break code,
                Ok((None, state)) => {
                    if unreachable_polls > 0 {
                        sink.note("login node reachable again");
                    }
                    unreachable_polls = 0;
                    state
                }
                Err(error) => {
                    unreachable_polls += 1;
                    if unreachable_polls == 1 {
                        sink.error(&format!("{error} - the job keeps running; retrying"));
                    }
                    if unreachable_polls >= MAX_UNREACHABLE_POLLS {
                        return Err(ProviderError::Unreachable(format!(
                            "lost contact with {} for {} polls; slurm job {slurm_id} was left running in {workdir}",
                            self.config.ssh.host, unreachable_polls
                        )));
                    }
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            };
            if state.is_empty() {
                // Gone from the queue with no sentinel written: the job was killed before
                // its script could finish (OOM, timeout, node failure). Give the shared
                // filesystem a couple of polls to catch up before calling it.
                missing_polls += 1;
                if missing_polls >= 3 {
                    let reason = slurm_reason(&remote, &slurm_id).await;
                    sink.error(&format!(
                        "slurm job {slurm_id} ended without completing: {reason}"
                    ));
                    break -1;
                }
            } else {
                missing_polls = 0;
                if state != last_state {
                    sink.note(&format!("slurm state: {state}"));
                    last_state = state;
                }
            }

            tokio::time::sleep(POLL).await;
        };

        if let Ok((chunk, _)) = remote.tail_from(&out_path, offset).await {
            if !chunk.is_empty() {
                sink.write_bytes(chunk.replace('\n', "\r\n").as_bytes());
            }
        }

        sink.note("fetching results");
        if let Err(error) = remote.download(&workdir, Path::new(&spec.local_dir)).await {
            sink.error(&format!("{error}"));
            if exit_code == 0 {
                return Err(error);
            }
        }

        Ok(RunOutcome {
            exit_code,
            external_id: Some(format!("slurm {slurm_id}")),
        })
    }
}

/// The job id out of `sbatch --parsable`: the last line, before any `;cluster`.
///
/// Only digits are taken. The id goes unquoted into `scancel`, `squeue` and `sacct`
/// commands run on the login node, and a site's sbatch wrapper or a login banner that
/// printed something else on the last line must not become part of a command.
fn parse_sbatch_id(stdout: &str) -> Option<String> {
    let id = stdout
        .trim()
        .lines()
        .last()?
        .split(';') // `--parsable` appends `;cluster` on federated setups
        .next()?
        .trim();
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

fn sinfo_command(partition: Option<&str>) -> String {
    match partition {
        Some(partition) => format!(
            "sinfo -h -p {} -o '%G|%D' 2>/dev/null || true",
            shell_quote(partition)
        ),
        None => "sinfo -h -o '%G|%D' 2>/dev/null || true".to_string(),
    }
}

/// Reads GPU types out of `sinfo`'s gres column, e.g. `gpu:a100:4(S:0-1)`.
fn parse_sinfo_gres(stdout: &str) -> Vec<super::probe::Gpu> {
    use super::probe::{Gpu, GpuBackend};
    let mut seen = std::collections::BTreeSet::new();
    let mut gpus = Vec::new();
    for line in stdout.lines() {
        let gres = line.split('|').next().unwrap_or("").trim();
        if gres.is_empty() || gres == "(null)" {
            continue;
        }
        for entry in gres.split(',') {
            // A topology suffix such as `(S:0-1)` has a colon of its own, so it comes off
            // before the entry is split into fields.
            let entry = entry.trim();
            let entry = entry.split('(').next().unwrap_or(entry);
            let mut fields = entry.split(':');
            if fields.next() != Some("gpu") {
                continue;
            }
            let Some(second) = fields.next() else {
                continue;
            };
            // The count field carries topology suffixes like `4(S:0-1)`; only the leading
            // digits are the count.
            let leading_count = |field: &str| -> Option<u32> {
                field
                    .chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
            };
            // `gpu:a100:4` names a model; `gpu:4` does not, and its second field is the
            // count rather than a card called "4".
            let (model, count) = match fields.next() {
                Some(third) => (second, leading_count(third).unwrap_or(1)),
                None => match leading_count(second) {
                    Some(count) => ("gpu", count),
                    None => (second, 1),
                },
            };
            let name = format!("{model} x{count} (partition)");
            if seen.insert(name.clone()) {
                gpus.push(Gpu {
                    backend: GpuBackend::Cuda,
                    name,
                    memory_gb: None,
                    driver: None,
                });
            }
        }
    }
    gpus
}

/// Best-effort explanation for a job that vanished. `sacct` is often the only thing that
/// knows it was an OOM kill rather than a crash.
async fn slurm_reason(remote: &Remote, slurm_id: &str) -> String {
    let command = format!(
        "sacct -j {slurm_id} --format=State,ExitCode,Reason --noheader --parsable2 2>/dev/null | head -1 || true"
    );
    match remote.run_quiet(&command, Duration::from_secs(30)).await {
        Ok(captured) if !captured.stdout.trim().is_empty() => captured.stdout.trim().to_string(),
        _ => "no accounting record available".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gpu_models_from_sinfo() {
        let gpus = parse_sinfo_gres("gpu:a100:4(S:0-1)|12\ngpu:a100:4(S:0-1)|8\ngpu:h100:8|2\n");
        let names: Vec<&str> = gpus.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["a100 x4 (partition)", "h100 x8 (partition)"]);
    }

    #[test]
    fn a_gres_without_a_model_is_a_count() {
        let gpus = parse_sinfo_gres("gpu:4|3\ngpu:2(S:0)|1\n");
        let names: Vec<&str> = gpus.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["gpu x4 (partition)", "gpu x2 (partition)"]);
    }

    #[test]
    fn only_a_number_is_taken_as_a_job_id() {
        assert_eq!(parse_sbatch_id("4821\n").as_deref(), Some("4821"));
        assert_eq!(
            parse_sbatch_id("Submitted to the queue\n4821;cluster-a\n").as_deref(),
            Some("4821")
        );
        assert_eq!(parse_sbatch_id(""), None);
        assert_eq!(parse_sbatch_id("4821; rm -rf ~"), Some("4821".into()));
        assert_eq!(parse_sbatch_id("$(reboot)"), None);
        assert_eq!(parse_sbatch_id("sbatch: error: invalid partition"), None);
    }

    fn provider(remote_dir: &str) -> SlurmProvider {
        let config: super::super::provider::ProviderConfig =
            serde_json::from_value(serde_json::json!({
                "kind": "slurm",
                "host": "cluster",
                "remote_dir": remote_dir,
                "partition": "gpu",
                "extra_sbatch": ["--qos=high"],
                "env": {"HF_HOME": "/data/my models; rm -rf ~", "bad name": "x"},
            }))
            .unwrap();
        let config = match config.tidied().unwrap() {
            super::super::provider::ProviderConfig::Slurm(config) => config,
            _ => unreachable!(),
        };
        SlurmProvider::new("slurm".into(), config)
    }

    fn spec(id: &str) -> JobSpec {
        JobSpec {
            id: id.into(),
            name: "run".into(),
            model_id: "m".into(),
            model_name: "M".into(),
            task: crate::models::Task::Custom,
            provider_id: "slurm".into(),
            world_id: None,
            local_dir: std::path::PathBuf::from("/tmp/run-abc"),
            steps: vec![],
            scalars: BTreeMap::new(),
            files: BTreeMap::new(),
            outputs: BTreeMap::new(),
            requirements: Default::default(),
            http: None,
        }
    }

    #[test]
    fn the_batch_script_quotes_what_it_runs_and_survives_a_short_id() {
        let provider = provider("/scratch/me");
        // A spec read back from an older database need not have a uuid for an id.
        let script = provider.sbatch_script(
            &spec("ab"),
            "/scratch/me/run-abc",
            &[".geis/step-01.sh".into()],
        );
        assert!(script.contains("#SBATCH --job-name=geis-ab\n"));
        assert!(script.contains("#SBATCH --partition=gpu\n"));
        assert!(script.contains("#SBATCH --qos=high\n"));
        assert!(script.contains("export HF_HOME='/data/my models; rm -rf ~'\n"));
        assert!(!script.contains("bad name"));
        assert!(script.contains("bash -l /scratch/me/run-abc/.geis/step-01.sh"));
        // Every #SBATCH line comes before the first command, or sbatch ignores the rest.
        let first_command = script
            .lines()
            .position(|line| line.starts_with("export"))
            .unwrap();
        assert!(script
            .lines()
            .skip(first_command)
            .all(|line| !line.starts_with("#SBATCH")));
        let script = provider.sbatch_script(&spec("가나다라마바사아자"), "/x", &[]);
        assert!(script.contains("--job-name=geis-\n"));
    }

    #[test]
    fn ignores_partitions_without_gpus() {
        assert!(parse_sinfo_gres("(null)|64\n").is_empty());
        assert!(parse_sinfo_gres("").is_empty());
    }
}
