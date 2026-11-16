//! A job is one execution of one model on one provider.
//!
//! Planning happens once, up front, and is host-agnostic: staged inputs are recorded as
//! paths *relative to the workdir*, and run-step templates are left unresolved. Each
//! provider then resolves them against whatever the workdir is on the host that will
//! actually execute - `/Users/me/...` locally, `/scratch/me/...` on a cluster. Without
//! that split, every remote provider would need to rewrite paths, and would get it wrong.

use crate::models::manifest::{HttpSpec, InputKind, Manifest, OutputSpec, Requirements, RunStep};
use crate::models::template::{shell_quote, Bindings, EnvMode};
use crate::models::Task;
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    /// Copying inputs to where they need to be.
    Staging,
    /// Submitted but not yet executing (slurm queue, HTTP backlog).
    Pending,
    Running,
    /// Pulling results back.
    Fetching,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Staging => "staging",
            JobState::Pending => "pending",
            JobState::Running => "running",
            JobState::Fetching => "fetching",
            JobState::Succeeded => "succeeded",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    /// Inverse of [`JobState::as_str`], for reading a row back out of the database.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => JobState::Queued,
            "staging" => JobState::Staging,
            "pending" => JobState::Pending,
            "running" => JobState::Running,
            "fetching" => JobState::Fetching,
            "succeeded" => JobState::Succeeded,
            "failed" => JobState::Failed,
            "cancelled" => JobState::Cancelled,
            _ => return None,
        })
    }
}

/// What the UI hands us: a model, a provider, and raw input values.
#[derive(Debug, Clone, Deserialize)]
pub struct JobRequest {
    pub model_id: String,
    pub provider_id: String,
    #[serde(default)]
    pub world_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
}

/// A validated, host-agnostic execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub model_name: String,
    pub task: Task,
    pub provider_id: String,
    pub world_id: Option<String>,
    /// Always on this machine: staging area, log file, and where artifacts end up.
    pub local_dir: PathBuf,
    pub steps: Vec<RunStep>,
    /// Validated scalar inputs, stringified for command interpolation.
    pub scalars: BTreeMap<String, String>,
    /// Input name -> path relative to the workdir on the executing host.
    pub files: BTreeMap<String, String>,
    pub outputs: BTreeMap<String, OutputSpec>,
    pub requirements: Requirements,
    pub http: Option<HttpSpec>,
}

impl JobSpec {
    pub const INPUT_SUBDIR: &'static str = "inputs";

    pub fn log_path(&self) -> PathBuf {
        self.local_dir.join("job.log")
    }

    /// Bindings for template resolution, given the workdir on the executing host.
    ///
    /// **Every value is already shell-quoted, so a manifest must not add quotes of its
    /// own.** Write `echo MARKER {{input.label}}`, not `echo "MARKER {{input.label}}"` -
    /// the latter produces `"MARKER 'from test'"` and the inner quotes become literal
    /// text. Quoting here rather than trusting manifest authors is the safe default: a
    /// pasted sequence or a path with a space in it must never become two arguments, and
    /// input text can never terminate a command.
    ///
    /// `{{input.x.raw}}` gives the unquoted value, for the rare case of interpolating
    /// something that is itself shell syntax - a list of flags, say.
    pub fn bindings(&self, workdir: &str, provider_vars: &BTreeMap<String, String>) -> Bindings {
        let mut bindings = Bindings::with_env_mode(EnvMode::ShellExpansion);
        bindings
            .set("job.id", shell_quote(&self.id))
            .set("job.name", shell_quote(&self.name))
            .set("job.name.raw", &self.name)
            .set("job.workdir", shell_quote(workdir))
            .set("job.workdir.raw", workdir)
            // Where this extension lives on the host it runs on. Left for that host's
            // shell to expand - its home is not this machine's - and safe unquoted: an id
            // is letters, digits, `-` and `_` by the manifest's own rule.
            //
            // An extension installed before the app was renamed is still in `~/.mini`, and
            // is used from there rather than installed again; anything new goes in `~/.geis`.
            .set(
                "ext.dir",
                format!(
                    "\"$(d=\"$HOME/.geis/extensions/{id}\"; [ -d \"$d\" ] || [ ! -d \"$HOME/.mini/extensions/{id}\" ] || d=\"$HOME/.mini/extensions/{id}\"; printf %s \"$d\")\"",
                    id = self.model_id
                ),
            );
        for (name, value) in &self.scalars {
            bindings.set(format!("input.{name}"), shell_quote(value));
            bindings.set(format!("input.{name}.raw"), value);
        }
        for (name, relative) in &self.files {
            let absolute = format!("{}/{}", workdir.trim_end_matches('/'), relative);
            bindings.set(format!("input.{name}.path"), shell_quote(&absolute));
            bindings.set(format!("input.{name}.raw"), &absolute);
            // Bare form too, so a manifest can write either.
            bindings.set(format!("input.{name}"), shell_quote(&absolute));
        }
        for (key, value) in provider_vars {
            bindings.set(format!("provider.{key}"), shell_quote(value));
            bindings.set(format!("provider.{key}.raw"), value);
        }
        bindings
    }

    /// Bindings for JSON contexts - HTTP request bodies.
    ///
    /// Shell quoting would produce `'MKTAYIAK'` inside a JSON string, so values here are
    /// JSON-escaped *without* the surrounding quotes: a manifest writes
    /// `{"sequence": "{{input.sequence}}"}` and gets valid JSON for any input, including
    /// multi-line FASTA. File inputs bind to their contents, because a remote service
    /// cannot see a local path.
    pub fn json_bindings(
        &self,
        provider_vars: &BTreeMap<String, String>,
    ) -> Result<Bindings, std::io::Error> {
        let mut bindings = Bindings::with_env_mode(EnvMode::LocalValue);
        bindings
            .set("job.id", json_escape(&self.id))
            .set("job.name", json_escape(&self.name))
            .set("model.id", json_escape(&self.model_id));
        for (name, value) in &self.scalars {
            bindings.set(format!("input.{name}"), json_escape(value));
        }
        for (name, relative) in &self.files {
            let path = self.local_dir.join(relative);
            let contents = std::fs::read_to_string(&path)?;
            bindings.set(format!("input.{name}"), json_escape(&contents));
            bindings.set(
                format!("input.{name}.path"),
                json_escape(&path.to_string_lossy()),
            );
        }
        for (key, value) in provider_vars {
            bindings.set(format!("provider.{key}"), json_escape(value));
        }
        Ok(bindings)
    }
}

/// JSON-escapes a string and strips the quotes serde adds, so the result can be dropped
/// inside a template's own quotes.
fn json_escape(value: &str) -> String {
    let quoted = serde_json::Value::String(value.to_string()).to_string();
    quoted[1..quoted.len() - 1].to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("input {0:?} is required")]
    Missing(String),
    #[error("input {name:?}: {message}")]
    BadValue { name: String, message: String },
    #[error("input {0:?} is not declared by this model")]
    Undeclared(String),
    #[error("preparing {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Validate a request against a manifest and stage its file inputs.
pub fn plan(
    manifest: &Manifest,
    request: &JobRequest,
    jobs_root: &Path,
) -> Result<JobSpec, PlanError> {
    for name in request.inputs.keys() {
        if !manifest.inputs.contains_key(name) {
            return Err(PlanError::Undeclared(name.clone()));
        }
    }

    let id = util::new_id();
    let name = request
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| manifest.name.clone());
    let local_dir = jobs_root.join(format!("{}-{}", util::slugify(&name), &id[..8]));
    let input_dir = local_dir.join(JobSpec::INPUT_SUBDIR);
    std::fs::create_dir_all(&input_dir).map_err(|source| PlanError::Io {
        path: input_dir.clone(),
        source,
    })?;

    // A request that is refused leaves nothing behind: without this every typo in a form
    // was another empty job directory that no job record points at.
    let staged = stage_inputs(manifest, request, &local_dir);
    let (scalars, files) = match staged {
        Ok(staged) => staged,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&local_dir);
            return Err(error);
        }
    };

    Ok(JobSpec {
        id,
        name,
        model_id: manifest.id.clone(),
        model_name: manifest.name.clone(),
        task: manifest.task,
        provider_id: request.provider_id.clone(),
        world_id: request.world_id.clone(),
        local_dir,
        steps: manifest.steps.clone(),
        scalars,
        files,
        outputs: manifest.outputs.clone(),
        requirements: manifest.requirements.clone(),
        http: manifest.http.clone(),
    })
}

type Staged = (BTreeMap<String, String>, BTreeMap<String, String>);

/// Validates each declared input and stages the file ones into `local_dir`.
fn stage_inputs(
    manifest: &Manifest,
    request: &JobRequest,
    local_dir: &Path,
) -> Result<Staged, PlanError> {
    let mut scalars = BTreeMap::new();
    let mut files = BTreeMap::new();

    for (input_name, spec) in &manifest.inputs {
        let supplied = request.inputs.get(input_name);
        let value = match (supplied, &spec.default) {
            (Some(value), _) if !value.is_null() => value.clone(),
            (_, Some(default)) => toml_to_json(default),
            _ if spec.required => return Err(PlanError::Missing(input_name.clone())),
            _ => continue,
        };

        let bad = |message: &str| PlanError::BadValue {
            name: input_name.clone(),
            message: message.to_string(),
        };

        match spec.kind {
            InputKind::Fasta => {
                let text = value
                    .as_str()
                    .ok_or_else(|| bad("expected sequence text"))?;
                let fasta = normalize_fasta(text, spec.fasta_header.as_deref())
                    .ok_or_else(|| bad("no residues found"))?;
                let relative = format!("{}/{input_name}.fasta", JobSpec::INPUT_SUBDIR);
                let path = local_dir.join(&relative);
                std::fs::write(&path, fasta).map_err(|source| PlanError::Io {
                    path: path.clone(),
                    source,
                })?;
                files.insert(input_name.clone(), relative);
            }
            InputKind::File => {
                let raw = value.as_str().ok_or_else(|| bad("expected a file path"))?;
                let source_path = PathBuf::from(raw);
                if !source_path.is_file() {
                    return Err(bad("file does not exist"));
                }
                if let Some(allowed) = spec.extensions.as_ref().filter(|a| !a.is_empty()) {
                    // A file with no extension is not one of the allowed kinds either.
                    let ext = source_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .unwrap_or_default();
                    if !allowed.iter().any(|a| a.to_ascii_lowercase() == ext) {
                        return Err(bad(&format!("expected one of: {}", allowed.join(", "))));
                    }
                }
                let filename = source_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("input");
                // Each input gets a directory of its own. Staged side by side, a receptor
                // and a ligand both called model.pdb are one file, and both bindings
                // point at whichever was copied last.
                let relative = format!("{}/{input_name}/{filename}", JobSpec::INPUT_SUBDIR);
                let dest = local_dir.join(&relative);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|source| PlanError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                std::fs::copy(&source_path, &dest).map_err(|source| PlanError::Io {
                    path: dest.clone(),
                    source,
                })?;
                files.insert(input_name.clone(), relative);
            }
            InputKind::Dir => {
                let raw = value.as_str().ok_or_else(|| bad("expected a directory"))?;
                if !Path::new(raw).is_dir() {
                    return Err(bad("directory does not exist"));
                }
                // Directories are referenced in place rather than copied; a weights dir
                // can be hundreds of gigabytes.
                scalars.insert(input_name.clone(), raw.to_string());
            }
            InputKind::Int => {
                let number = value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
                    .ok_or_else(|| bad("expected an integer"))?;
                check_range(number as f64, spec.min, spec.max).map_err(|m| bad(&m))?;
                scalars.insert(input_name.clone(), number.to_string());
            }
            InputKind::Float => {
                let number = value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
                    .ok_or_else(|| bad("expected a number"))?;
                check_range(number, spec.min, spec.max).map_err(|m| bad(&m))?;
                scalars.insert(input_name.clone(), number.to_string());
            }
            InputKind::Bool => {
                let flag = value
                    .as_bool()
                    .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                    .ok_or_else(|| bad("expected true or false"))?;
                scalars.insert(input_name.clone(), flag.to_string());
            }
            InputKind::Enum => {
                let choice = value.as_str().ok_or_else(|| bad("expected a choice"))?;
                let choices = spec.choices.as_deref().unwrap_or_default();
                if !choices.iter().any(|c| c == choice) {
                    return Err(bad(&format!("expected one of: {}", choices.join(", "))));
                }
                scalars.insert(input_name.clone(), choice.to_string());
            }
            InputKind::Text => {
                let text = value.as_str().ok_or_else(|| bad("expected text"))?;
                scalars.insert(input_name.clone(), text.to_string());
            }
        }
    }

    Ok((scalars, files))
}

fn check_range(value: f64, min: Option<f64>, max: Option<f64>) -> Result<(), String> {
    // `"nan".parse()` succeeds, and NaN compares false with every bound, so without this
    // it passes any range and reaches the tool as the text "NaN".
    if !value.is_finite() {
        return Err("must be a finite number".into());
    }
    if let Some(min) = min {
        if value < min {
            return Err(format!("must be >= {min}"));
        }
    }
    if let Some(max) = max {
        if value > max {
            return Err(format!("must be <= {max}"));
        }
    }
    Ok(())
}

fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Value::from(*f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        other => serde_json::Value::String(other.to_string()),
    }
}

/// Accepts either a raw sequence or FASTA and always emits valid FASTA.
///
/// Users paste both, and every downstream tool wants FASTA. Doing this at plan time means
/// no engine wrapper has to care.
///
/// `header` is what a bare sequence is titled with. Most tools ignore the title; a few
/// parse it - Boltz reads `CHAIN|ENTITY_TYPE` out of it and rejects anything else - so the
/// manifest may say what its engine expects.
fn normalize_fasta(input: &str, header: Option<&str>) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('>') {
        let has_residues = trimmed
            .lines()
            .filter(|l| !l.starts_with('>'))
            .any(|l| l.chars().any(|c| c.is_ascii_alphabetic()));
        return has_residues.then(|| format!("{}\n", trimmed.replace("\r\n", "\n")));
    }
    let residues: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == '*' || *c == '-')
        .collect::<String>()
        .to_ascii_uppercase();
    if residues.is_empty() {
        return None;
    }
    let wrapped = residues
        .as_bytes()
        .chunks(60)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let header = header
        .map(|h| h.trim().trim_start_matches('>'))
        .filter(|h| !h.is_empty() && !h.contains('\n'))
        .unwrap_or("query");
    Some(format!(">{header}\n{wrapped}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_sequence_becomes_fasta() {
        let out = normalize_fasta("mkt aylk\nqil", None).unwrap();
        assert_eq!(out, ">query\nMKTAYLKQIL\n");
    }

    #[test]
    fn a_bare_sequence_takes_the_title_the_engine_asked_for() {
        let out = normalize_fasta("mktaylk", Some("A|protein")).unwrap();
        assert_eq!(out, ">A|protein\nMKTAYLK\n");
        // A title the user wrote is theirs; the manifest's is only a default.
        let out = normalize_fasta(">B|protein\nMKT", Some("A|protein")).unwrap();
        assert!(out.starts_with(">B|protein\n"));
        // An empty one is no title at all.
        assert!(normalize_fasta("mkt", Some("  "))
            .unwrap()
            .starts_with(">query\n"));
    }

    #[test]
    fn existing_fasta_is_preserved() {
        let out = normalize_fasta(">p53 tumor\nMEEPQSD", None).unwrap();
        assert!(out.starts_with(">p53 tumor\n"));
    }

    #[test]
    fn header_without_residues_is_rejected() {
        assert!(normalize_fasta(">lonely header", None).is_none());
        assert!(normalize_fasta("   ", None).is_none());
    }

    #[test]
    fn long_sequences_wrap_at_60() {
        let out = normalize_fasta(&"A".repeat(130), None).unwrap();
        let body: Vec<&str> = out.lines().skip(1).collect();
        assert_eq!(body.len(), 3);
        assert_eq!(body[0].len(), 60);
        assert_eq!(body[2].len(), 10);
    }

    #[test]
    fn json_bindings_survive_multiline_fasta() {
        let escaped = json_escape(">query\nMKT\"AYL");
        let body = format!("{{\"seq\": \"{escaped}\"}}");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(parsed["seq"], ">query\nMKT\"AYL");
    }

    fn manifest(inputs: &str) -> Manifest {
        let text = format!(
            "schema = 1\nid = \"demo\"\nname = \"Demo run\"\ntask = \"custom\"\n[[run]]\ncommand = \"true\"\n[inputs]\n{inputs}"
        );
        toml::from_str(&text).unwrap()
    }

    fn request(inputs: serde_json::Value) -> JobRequest {
        serde_json::from_value(serde_json::json!({
            "model_id": "demo", "provider_id": "local", "inputs": inputs,
        }))
        .unwrap()
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("geis-plan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inputs_are_checked_against_what_the_manifest_declares() {
        let root = scratch();
        let m = manifest(
            "n = { type = \"int\", min = 1, max = 10, default = 3 }\n\
             x = { type = \"float\", min = 0.0, max = 1.0 }\n\
             mode = { type = \"enum\", choices = [\"a\", \"b\"] }\n\
             flag = { type = \"bool\" }\n\
             seq = { type = \"fasta\", required = true }\n",
        );
        let ok = plan(
            &m,
            &request(serde_json::json!({"seq": "MKT", "x": "0.5", "flag": "true"})),
            &root,
        )
        .unwrap();
        assert_eq!(ok.scalars["n"], "3", "the default fills in");
        assert_eq!(ok.scalars["x"], "0.5");
        assert_eq!(ok.files["seq"], "inputs/seq.fasta");
        assert!(ok.local_dir.join("inputs/seq.fasta").is_file());
        assert!(ok.local_dir.starts_with(&root));

        let refused = |inputs: serde_json::Value| plan(&m, &request(inputs), &root).is_err();
        assert!(
            refused(serde_json::json!({})),
            "a required input is missing"
        );
        assert!(
            refused(serde_json::json!({"seq": "MKT", "nope": 1})),
            "undeclared"
        );
        assert!(refused(serde_json::json!({"seq": "MKT", "n": 11})));
        assert!(refused(serde_json::json!({"seq": "MKT", "n": 2.5})));
        assert!(refused(serde_json::json!({"seq": "MKT", "n": "3; reboot"})));
        assert!(
            refused(serde_json::json!({"seq": "MKT", "x": "NaN"})),
            "NaN slips past every bound"
        );
        assert!(refused(serde_json::json!({"seq": "MKT", "x": "inf"})));
        assert!(refused(serde_json::json!({"seq": "MKT", "mode": "c"})));
        assert!(refused(serde_json::json!({"seq": "MKT", "flag": "yes"})));
        assert!(
            refused(serde_json::json!({"seq": "1234 !!"})),
            "no residues"
        );
        assert!(refused(serde_json::json!({"seq": 42})));
        // Only the job that was accepted has a directory.
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_job_name_cannot_steer_where_the_job_is_written() {
        let root = scratch();
        let m = manifest("t = { type = \"text\" }\n");
        let mut req = request(serde_json::json!({"t": "it's; $(id)\n`x`"}));
        req.name = Some("../../../etc".into());
        let spec = plan(&m, &req, &root).unwrap();
        assert_eq!(spec.local_dir.parent(), Some(root.as_path()));
        assert!(spec
            .local_dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("etc-"));
        // Text input reaches a command as one quoted word, and raw only when asked for.
        let bindings = spec.bindings("/w", &BTreeMap::new());
        assert_eq!(bindings.get("input.t"), Some("'it'\\''s; $(id)\n`x`'"));
        assert_eq!(bindings.get("input.t.raw"), Some("it's; $(id)\n`x`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn file_inputs_are_staged_apart_and_keep_their_names() {
        let root = scratch();
        let source = root.join("my structures");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("model 1.PDB"), b"ATOM").unwrap();
        std::fs::write(source.join("noext"), b"ATOM").unwrap();
        let m = manifest(
            "a = { type = \"file\", extensions = [\"pdb\"] }\nb = { type = \"file\", extensions = [\"pdb\"] }\n",
        );
        let path = source.join("model 1.PDB").to_string_lossy().to_string();
        let spec = plan(
            &m,
            &request(serde_json::json!({"a": path, "b": path})),
            &root,
        )
        .unwrap();
        assert_eq!(spec.files["a"], "inputs/a/model 1.PDB");
        assert_eq!(spec.files["b"], "inputs/b/model 1.PDB");
        let bindings = spec.bindings("/scratch/run", &BTreeMap::new());
        assert_eq!(
            bindings.get("input.a"),
            Some("'/scratch/run/inputs/a/model 1.PDB'")
        );

        let noext = source.join("noext").to_string_lossy().to_string();
        assert!(plan(&m, &request(serde_json::json!({"a": noext})), &root).is_err());
        let missing = source.join("gone.pdb").to_string_lossy().to_string();
        assert!(plan(&m, &request(serde_json::json!({"a": missing})), &root).is_err());
        assert!(
            plan(
                &m,
                &request(serde_json::json!({"a": source.to_string_lossy()})),
                &root
            )
            .is_err(),
            "a directory is not a file"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn json_bindings_carry_file_contents_and_fail_cleanly_without_them() {
        let root = scratch();
        let m = manifest("seq = { type = \"fasta\" }\n");
        let spec = plan(&m, &request(serde_json::json!({"seq": "MKT\"\\"})), &root).unwrap();
        let bindings = spec.json_bindings(&BTreeMap::new()).unwrap();
        let body = format!("{{\"s\": \"{}\"}}", bindings.get("input.seq").unwrap());
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["s"], ">query\nMKT\n");
        std::fs::remove_dir_all(&spec.local_dir).unwrap();
        assert!(spec.json_bindings(&BTreeMap::new()).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn job_states_round_trip_and_unknown_ones_are_none() {
        for state in [
            JobState::Queued,
            JobState::Staging,
            JobState::Pending,
            JobState::Running,
            JobState::Fetching,
            JobState::Succeeded,
            JobState::Failed,
            JobState::Cancelled,
        ] {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        assert_eq!(JobState::parse("RUNNING"), None);
        assert_eq!(JobState::parse(""), None);
    }

    #[test]
    fn remote_workdir_rewrites_every_input_path() {
        let spec = JobSpec {
            id: "abc".into(),
            name: "run".into(),
            model_id: "boltz2".into(),
            model_name: "Boltz-2".into(),
            task: Task::StructurePrediction,
            provider_id: "cluster".into(),
            world_id: None,
            local_dir: PathBuf::from("/Users/me/jobs/run-abc"),
            steps: vec![],
            scalars: BTreeMap::from([("recycles".into(), "3".into())]),
            files: BTreeMap::from([("sequence".into(), "inputs/sequence.fasta".into())]),
            outputs: BTreeMap::new(),
            requirements: Requirements::default(),
            http: None,
        };
        let bindings = spec.bindings("/scratch/me/run-abc", &BTreeMap::new());
        assert_eq!(
            bindings.get("input.sequence.path"),
            Some("/scratch/me/run-abc/inputs/sequence.fasta")
        );
        assert_eq!(bindings.get("job.workdir"), Some("/scratch/me/run-abc"));
        assert_eq!(bindings.get("input.recycles"), Some("3"));
        assert_eq!(
            bindings.get("input.sequence.raw"),
            Some("/scratch/me/run-abc/inputs/sequence.fasta"),
            "the .raw form must skip quoting"
        );
    }
}
