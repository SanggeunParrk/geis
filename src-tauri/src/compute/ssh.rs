//! Runs jobs on another host over ssh - a lab workstation or a GPU box.

use super::exec::{self, CancelToken, ExecRequest, LogSink};
use super::job::JobSpec;
use super::probe::{HostProbe, PROBE_SCRIPT};
use super::provider::{Provider, ProviderError, ProviderKind, RunOutcome, SshConfig};
use super::remote::{remote_workdir, Remote};
use super::script;
use crate::models::template::shell_quote;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub struct SshProvider {
    id: String,
    config: SshConfig,
}

impl SshProvider {
    pub fn new(id: String, config: SshConfig) -> Self {
        Self { id, config }
    }

    fn remote(&self) -> Remote {
        Remote::new(self.config.host.clone(), self.config.ssh_options.clone())
    }
}

/// Writes the generated step scripts into the workdir before it is uploaded.
///
/// Shipping scripts as files rather than piping them through `ssh` sidesteps every layer
/// of quoting between the local shell, ssh, and the remote shell - and leaves an exact
/// record in the workdir of what ran, which is the first thing anyone wants when a run
/// fails on a cluster.
pub fn write_step_scripts(
    spec: &JobSpec,
    workdir_on_host: &str,
    provider_vars: &BTreeMap<String, String>,
    prelude: &[String],
) -> Result<Vec<String>, ProviderError> {
    let script_dir = spec.local_dir.join(".geis");
    std::fs::create_dir_all(&script_dir)?;
    let bindings = spec.bindings(workdir_on_host, provider_vars);
    let mut relative_paths = Vec::new();

    for (index, step) in spec.steps.iter().enumerate() {
        let body = script::step_script(step, workdir_on_host, &bindings, prelude)?;
        let name = format!("step-{:02}.sh", index + 1);
        std::fs::write(script_dir.join(&name), body)?;
        relative_paths.push(format!(".geis/{name}"));
    }
    Ok(relative_paths)
}

pub fn env_prefix(env: &BTreeMap<String, String>) -> String {
    if env.is_empty() {
        return String::new();
    }
    let assignments: Vec<String> = env
        .iter()
        .filter(|(key, _)| script::is_env_name(key))
        .map(|(key, value)| format!("{key}={}", shell_quote(value)))
        .collect();
    if assignments.is_empty() {
        return String::new();
    }
    format!("env {} ", assignments.join(" "))
}

/// What ssh is asked to run, as one word for the local shell.
///
/// ssh joins its arguments with spaces and the remote login shell parses the result again,
/// so quoting each piece for the local shell alone leaves a value with a space, a `;` or a
/// `$(...)` bare on the far side. The whole remote command line is therefore quoted once
/// more: the local shell strips the outer layer, the remote one strips the inner.
pub fn remote_command(env: &str, script_path: &str) -> String {
    shell_quote(&format!("{env}bash -l {}", shell_quote(script_path)))
}

#[async_trait::async_trait]
impl Provider for SshProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ssh
    }

    fn vars(&self) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        vars.insert("host".into(), self.config.host.clone());
        vars.insert("remote_dir".into(), self.config.remote_dir.clone());
        if let Some(scratch) = &self.config.exec.scratch {
            vars.insert("scratch".into(), scratch.clone());
        }
        if let Some(weights) = &self.config.exec.weights_dir {
            vars.insert("weights_dir".into(), weights.clone());
        }
        vars
    }

    async fn probe(&self) -> Result<HostProbe, ProviderError> {
        let captured = self
            .remote()
            .capture_script(PROBE_SCRIPT, Duration::from_secs(90))
            .await?;
        let probe = HostProbe::parse(&captured.stdout);
        if !probe.complete {
            return Err(ProviderError::Unreachable(format!(
                "probe of {} did not finish: {}",
                self.config.host,
                captured.message()
            )));
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
        let workdir = remote_workdir(&self.config.remote_dir, &spec.local_dir);

        sink.note(&format!("staging to {}:{}", self.config.host, workdir));
        remote.mkdir(&workdir).await?;
        let scripts = write_step_scripts(spec, &workdir, &self.vars(), &self.config.exec.prelude)?;
        remote.upload(&spec.local_dir, &workdir).await?;

        let env = env_prefix(&self.config.exec.env);
        let total = scripts.len();
        let mut exit_code = 0;

        for (index, relative) in scripts.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            sink.note(&script::step_label(index, total, &spec.steps[index], spec));
            // -tt forces a remote tty. Without it, killing the local ssh leaves the
            // remote process running: cancelling a job in the UI would appear to work
            // while a GPU stayed busy for another hour.
            let command = format!(
                "ssh -tt {} {} {}",
                shell_quote_all(&self.config.ssh_options),
                shell_quote(&self.config.host),
                remote_command(&env, &format!("{workdir}/{relative}"))
            );
            exit_code = exec::run(
                ExecRequest {
                    command,
                    cwd: spec.local_dir.clone(),
                    env: BTreeMap::new(),
                },
                sink.clone(),
                cancel.clone(),
            )
            .await?;
            if exit_code != 0 {
                sink.error(&format!("remote step {} exited {exit_code}", index + 1));
                break;
            }
        }

        // Results are fetched even after a failure: partial output and whatever the tool
        // wrote before dying are usually what explains the failure.
        sink.note("fetching results");
        if let Err(error) = remote.download(&workdir, Path::new(&spec.local_dir)).await {
            sink.error(&format!("{error}"));
            if exit_code == 0 {
                return Err(error);
            }
        }

        Ok(RunOutcome {
            exit_code,
            external_id: Some(format!("{}:{}", self.config.host, workdir)),
        })
    }
}

fn shell_quote_all(options: &[String]) -> String {
    let mut parts = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
    ];
    parts.extend(options.iter().map(|o| shell_quote(o)));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a POSIX shell makes of one quoted word: the inverse of `shell_quote`, enough
    /// of it to check a round trip.
    fn unquote(word: &str) -> String {
        let mut out = String::new();
        let mut chars = word.chars().peekable();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('\'', _) => quoted = !quoted,
                ('\\', false) => out.extend(chars.next()),
                (c, _) => out.push(c),
            }
        }
        out
    }

    #[test]
    fn the_remote_command_survives_both_shells() {
        let mut env = BTreeMap::new();
        env.insert(
            "HF_HOME".to_string(),
            "/data/my models; rm -rf ~".to_string(),
        );
        let word = remote_command(&env_prefix(&env), "/scratch/it's here/.geis/step-01.sh");
        // The local shell sees exactly one word, so ssh is handed one argument...
        assert!(word.starts_with('\'') && word.ends_with('\''));
        // ...and what the remote shell then parses still has every value quoted.
        let remote = unquote(&word);
        assert_eq!(
            remote,
            r"env HF_HOME='/data/my models; rm -rf ~' bash -l '/scratch/it'\''s here/.geis/step-01.sh'"
        );
    }

    #[test]
    fn a_name_that_is_not_a_name_is_not_exported() {
        let mut env = BTreeMap::new();
        env.insert("OK_1".to_string(), "1".to_string());
        env.insert("x; reboot".to_string(), "1".to_string());
        assert_eq!(env_prefix(&env), "env OK_1=1 ");
    }
}
