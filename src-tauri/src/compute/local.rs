//! Runs jobs on this machine.

use super::exec::{self, CancelToken, ExecRequest, LogSink};
use super::job::JobSpec;
use super::probe::{HostProbe, PROBE_SCRIPT};
use super::provider::{LocalConfig, Provider, ProviderError, ProviderKind, RunOutcome};
use super::script;
use std::collections::BTreeMap;
use std::time::Duration;

pub struct LocalProvider {
    id: String,
    config: LocalConfig,
}

impl LocalProvider {
    pub fn new(id: String, config: LocalConfig) -> Self {
        Self { id, config }
    }
}

#[async_trait::async_trait]
impl Provider for LocalProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Local
    }

    fn vars(&self) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        if let Some(scratch) = &self.config.exec.scratch {
            vars.insert("scratch".into(), scratch.clone());
        }
        if let Some(weights) = &self.config.exec.weights_dir {
            vars.insert("weights_dir".into(), weights.clone());
        }
        vars
    }

    async fn probe(&self) -> Result<HostProbe, ProviderError> {
        let captured = exec::capture("sh -s", Some(PROBE_SCRIPT), Duration::from_secs(60)).await?;
        let probe = HostProbe::parse(&captured.stdout);
        if !probe.complete {
            return Err(ProviderError::Unreachable(format!(
                "host probe did not finish: {}",
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
        let workdir = spec
            .local_dir
            .to_str()
            .ok_or_else(|| ProviderError::Setup("workdir is not valid UTF-8".into()))?
            .to_string();
        let bindings = spec.bindings(&workdir, &self.vars());
        let total = spec.steps.len();

        for (index, step) in spec.steps.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            sink.note(&script::step_label(index, total, step, spec));
            let command =
                script::step_script(step, &workdir, &bindings, &self.config.exec.prelude)?;
            let code = exec::run(
                ExecRequest {
                    command,
                    cwd: spec.local_dir.clone(),
                    env: script::step_env(step, &self.config.exec.env),
                },
                sink.clone(),
                cancel.clone(),
            )
            .await?;
            if code != 0 {
                sink.error(&format!("step {} exited {code}", index + 1));
                return Ok(RunOutcome {
                    exit_code: code,
                    external_id: None,
                });
            }
        }

        Ok(RunOutcome {
            exit_code: 0,
            external_id: None,
        })
    }
}
