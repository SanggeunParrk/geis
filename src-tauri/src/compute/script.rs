//! Assembles the shell script for one run step.
//!
//! Shared by every shell-executing provider so that a manifest behaves identically
//! locally, over ssh, and inside an sbatch script.

use super::job::JobSpec;
use crate::models::manifest::RunStep;
use crate::models::template::{resolve, shell_quote, Bindings, TemplateError};
use std::collections::BTreeMap;

/// Full script for a step: fail-fast, provider prelude, then the resolved command.
///
/// `set -e` applies to the prelude too. That is deliberate - a `module load cuda` that
/// silently fails produces a CUDA error thousands of log lines later, and blaming the
/// prelude immediately is far more useful.
pub fn step_script(
    step: &RunStep,
    workdir: &str,
    bindings: &Bindings,
    prelude: &[String],
) -> Result<String, TemplateError> {
    let mut script = String::from("set -e\n");
    // The step's own environment is written into the script rather than left to whoever
    // launches it. The local provider can set a child's environment directly, but ssh and
    // sbatch cannot, and a variable that only exists on one kind of provider is a manifest
    // that works on a laptop and fails on the cluster with no hint why.
    for (key, value) in &step.env {
        if is_env_name(key) {
            script.push_str(&format!("export {key}={}\n", shell_quote(value)));
        }
    }
    for line in prelude {
        script.push_str(&resolve(line, bindings)?);
        script.push('\n');
    }
    let cwd = match &step.cwd {
        Some(relative) => format!("{}/{}", workdir.trim_end_matches('/'), relative),
        None => workdir.to_string(),
    };
    script.push_str(&format!("cd {}\n", shell_quote(&cwd)));
    script.push_str(&resolve(&step.command, bindings)?);
    script.push('\n');
    Ok(script)
}

/// A name a POSIX shell will accept on the left of an assignment. Anything else would be
/// read as a command, so it is left out rather than run.
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Environment for a step: provider-wide settings first, then anything the manifest
/// declared for this step.
pub fn step_env(
    step: &RunStep,
    provider_env: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = provider_env.clone();
    env.extend(step.env.clone());
    env
}

/// Label shown in the terminal panel for a step.
pub fn step_label(index: usize, total: usize, step: &RunStep, spec: &JobSpec) -> String {
    let name = step
        .label
        .clone()
        .unwrap_or_else(|| format!("step {}", index + 1));
    format!("[{}/{}] {} - {}", index + 1, total, spec.model_name, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::manifest::StepKind;

    fn step(command: &str, cwd: Option<&str>, env: &[(&str, &str)]) -> RunStep {
        RunStep {
            kind: StepKind::Shell,
            command: command.into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cwd: cwd.map(str::to_string),
            label: None,
        }
    }

    #[test]
    fn the_script_quotes_what_it_was_given_and_drops_what_is_not_a_name() {
        let mut bindings = Bindings::new();
        bindings.set("job.workdir", shell_quote("/scratch/my run"));
        let script = step_script(
            &step(
                "tool --out {{job.workdir}}",
                Some("sub dir"),
                &[
                    ("TOKEN", "a'b $(id)\nc"),
                    ("1BAD", "x"),
                    ("A;B", "y"),
                    ("", "z"),
                ],
            ),
            "/scratch/my run/",
            &bindings,
            &["module load cuda".into()],
        )
        .unwrap();
        assert_eq!(
            script,
            "set -e\nexport TOKEN='a'\\''b $(id)\nc'\nmodule load cuda\ncd '/scratch/my run/sub dir'\ntool --out '/scratch/my run'\n"
        );
    }

    #[test]
    fn an_unknown_placeholder_in_the_prelude_fails_the_step() {
        let result = step_script(
            &step("true", None, &[]),
            "/w",
            &Bindings::new(),
            &["source {{provider.nope}}".into()],
        );
        assert!(matches!(result, Err(TemplateError::Unknown(_))));
    }

    #[test]
    fn environment_names_are_what_a_shell_accepts() {
        for good in ["A", "_x", "CUDA_VISIBLE_DEVICES", "a1"] {
            assert!(is_env_name(good), "{good}");
        }
        for bad in ["", "1A", "A-B", "A B", "A=B", "단", "A\n"] {
            assert!(!is_env_name(bad), "{bad:?}");
        }
    }
}
