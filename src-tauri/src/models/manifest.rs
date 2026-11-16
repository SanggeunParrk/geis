//! Declarative model description.
//!
//! The compute core knows nothing about proteins. A model is a TOML file: what it needs,
//! what it takes in, what it leaves behind, and the commands that produce it. Adding an
//! engine is writing one of these - never Rust.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    StructurePrediction,
    SequenceDesign,
    Docking,
    Embedding,
    Msa,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuNeed {
    Required,
    Recommended,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Runs in a python environment the provider points at.
    Python,
    /// Runs inside a container image.
    Docker,
    /// A binary on PATH.
    Binary,
    /// No local execution at all - the model lives behind an HTTP endpoint.
    Http,
}

/// How model weights reach the machine that will run inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeightsPolicy {
    /// Downloadable without gating; the run command handles it.
    Auto,
    /// User must obtain them separately and accept terms. The UI blocks the model until
    /// `weights_dir` is set for it. AlphaFold 3 is the reason this variant exists.
    Manual,
    /// Nothing to fetch.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default = "default_gpu")]
    pub gpu: GpuNeed,
    /// Minimum VRAM in GiB for a comfortable run. Advisory - used to warn, not to block.
    pub vram_gb: Option<u32>,
    #[serde(default = "default_runtime")]
    pub runtime: RuntimeKind,
    #[serde(default = "default_weights")]
    pub weights: WeightsPolicy,
    pub disk_gb: Option<u32>,
    /// Executables that must resolve on the target host before we bother submitting.
    #[serde(default)]
    pub needs_binaries: Vec<String>,
    /// Shown verbatim in the model card - terms, citation asks, caveats.
    pub notes: Option<String>,
}

fn default_gpu() -> GpuNeed {
    GpuNeed::Recommended
}
fn default_runtime() -> RuntimeKind {
    RuntimeKind::Python
}
fn default_weights() -> WeightsPolicy {
    WeightsPolicy::Auto
}

impl Default for Requirements {
    fn default() -> Self {
        Self {
            gpu: default_gpu(),
            vram_gb: None,
            runtime: default_runtime(),
            weights: default_weights(),
            disk_gb: None,
            needs_binaries: Vec::new(),
            notes: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// One or more sequences. Staged to a .fasta file; `{{input.x.path}}` is that file.
    Fasta,
    /// An existing file the user picks. Staged (copied/uploaded) before the run.
    File,
    Dir,
    Int,
    Float,
    Bool,
    /// Constrained to `choices`.
    Enum,
    Text,
}

/// One declared parameter. Flat on purpose so a TOML inline table reads naturally:
/// `recycles = { type = "int", default = 3, min = 1, max = 10 }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSpec {
    #[serde(rename = "type")]
    pub kind: InputKind,
    #[serde(default)]
    pub required: bool,
    pub default: Option<toml::Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub choices: Option<Vec<String>>,
    /// Accepted extensions for `File` inputs, without the dot.
    pub extensions: Option<Vec<String>>,
    /// For `Fasta` inputs: the title given to a sequence pasted without one. Only
    /// engines that parse the title need it.
    #[serde(default)]
    pub fasta_header: Option<String>,
    pub label: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    /// Loadable into a world.
    Structure,
    Json,
    Csv,
    Image,
    Log,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSpec {
    #[serde(rename = "type")]
    pub kind: OutputKind,
    /// Glob relative to the job workdir.
    pub pattern: String,
    pub label: Option<String>,
    /// Outputs marked primary are what the world loads by default.
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Shell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStep {
    #[serde(default = "default_step_kind")]
    pub kind: StepKind,
    /// Template string. See `crate::models::template` for the placeholder vocabulary.
    pub command: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Working directory for this step, relative to the job workdir. Defaults to it.
    pub cwd: Option<String>,
    pub label: Option<String>,
}

fn default_step_kind() -> StepKind {
    StepKind::Shell
}

/// For models served over HTTP rather than executed. Used only by the `http` provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpSpec {
    /// May itself contain `{{provider.endpoint}}`.
    pub submit_url: String,
    #[serde(default = "default_post")]
    pub submit_method: String,
    /// JSON body template.
    pub submit_body: Option<String>,
    /// JSON pointer into the submit response holding the job handle, e.g. `/id`.
    pub id_pointer: Option<String>,
    pub poll_url: Option<String>,
    /// JSON pointer to a status string, and the values that mean done/failed.
    pub status_pointer: Option<String>,
    #[serde(default)]
    pub done_values: Vec<String>,
    #[serde(default)]
    pub failed_values: Vec<String>,
    /// JSON pointer to a URL (or list of URLs) to download as artifacts.
    pub result_pointer: Option<String>,
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
}

fn default_post() -> String {
    "POST".into()
}
fn default_poll_secs() -> u64 {
    10
}

/// What kind of thing a manifest describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A predictor: something already on the host, pointed at and run.
    #[default]
    Model,
    /// A tool the user adds when they need it - TM-score, PyRosetta - and that this app
    /// installs for them, on whichever host it is to run on.
    Extension,
}

/// How an extension gets onto a host, and off it again.
///
/// Commands, like `[[run]]`: an extension is installed where it will run, which may be
/// this machine or a cluster over ssh, and the one thing every such place has is a shell.
/// Everything it installs goes under `{{ext.dir}}` - a directory of its own on that host -
/// so two extensions cannot disagree about a dependency and removing one is removing a
/// directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSpec {
    /// The commands that install it, in order. Each fails the install if it fails.
    pub steps: Vec<String>,
    /// Commands that remove it. Left out, the extension's directory is deleted.
    #[serde(default)]
    pub uninstall: Vec<String>,
    /// About how much it downloads, in megabytes, so nobody is surprised.
    pub download_mb: Option<u32>,
    /// What the user has to agree to or bring before installing - a licence, a key.
    pub terms: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    pub task: Task,
    pub version: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub citation: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub requirements: Requirements,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputSpec>,
    #[serde(default)]
    pub outputs: BTreeMap<String, OutputSpec>,
    #[serde(default, rename = "run")]
    pub steps: Vec<RunStep>,
    pub http: Option<HttpSpec>,
    /// How to install it. An extension has one; a model does not.
    pub install: Option<InstallSpec>,
    /// Set by the loader, not the file.
    #[serde(skip_deserializing, default)]
    pub source_path: Option<PathBuf>,
}

/// The two things done to an extension that are not running it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maintenance {
    Install,
    Remove,
}

impl Maintenance {
    /// What the job is called, which is also how its record is told from a run's.
    pub fn prefix(self) -> &'static str {
        match self {
            Maintenance::Install => "Install · ",
            Maintenance::Remove => "Remove · ",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut manifest: Manifest =
            toml::from_str(&text).map_err(|source| ManifestError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        manifest.source_path = Some(path.to_path_buf());
        manifest.validate(path)?;
        Ok(manifest)
    }

    fn validate(&self, path: &Path) -> Result<(), ManifestError> {
        let invalid = |message: String| ManifestError::Invalid {
            path: path.to_path_buf(),
            message,
        };
        if self.schema != SCHEMA {
            return Err(invalid(format!(
                "schema {} is not supported (this build understands {SCHEMA})",
                self.schema
            )));
        }
        if !is_plain_name(&self.id) {
            return Err(invalid(format!(
                "id {:?} must be non-empty and match [A-Za-z0-9_-]+",
                self.id
            )));
        }
        if self.requirements.runtime == RuntimeKind::Http {
            if self.http.is_none() {
                return Err(invalid("runtime = \"http\" needs an [http] section".into()));
            }
        } else if self.steps.is_empty() {
            return Err(invalid("no [[run]] steps declared".into()));
        }
        if let Some(http) = &self.http {
            // Polling needs something to poll with and a way to tell when to stop.
            // Without them the provider either skips the wait or never leaves it.
            if http.poll_url.is_some() {
                if http.id_pointer.is_none() {
                    return Err(invalid("[http] poll_url needs an id_pointer".into()));
                }
                if http.status_pointer.is_none() || http.done_values.is_empty() {
                    return Err(invalid(
                        "[http] poll_url needs a status_pointer and at least one done_values entry"
                            .into(),
                    ));
                }
            }
        }
        match (&self.kind, &self.install) {
            (Kind::Extension, None) => {
                return Err(invalid(
                    "kind = \"extension\" needs an [install] section".into(),
                ));
            }
            (Kind::Extension, Some(install)) if install.steps.is_empty() => {
                return Err(invalid("[install] has no steps".into()));
            }
            _ => {}
        }
        for (name, spec) in &self.inputs {
            // An input's name becomes a file name when it is staged (`inputs/<name>.fasta`)
            // and a placeholder key, so `../../x` would write outside the job's directory
            // and a dot would collide with the `.path` and `.raw` forms.
            if !is_plain_name(name) {
                return Err(invalid(format!(
                    "input name {name:?} must be non-empty and match [A-Za-z0-9_-]+"
                )));
            }
            if spec.kind == InputKind::Enum && spec.choices.as_ref().map_or(true, |c| c.is_empty())
            {
                return Err(invalid(format!(
                    "input {name:?} is an enum with no choices"
                )));
            }
        }
        for (name, output) in &self.outputs {
            // Patterns are matched under the job's directory and whatever matches is
            // recorded as the job's result. One that is absolute or climbs out with `..`
            // would pick up files that belong to nobody's job.
            let pattern = Path::new(&output.pattern);
            let escapes = pattern.is_absolute()
                || pattern.components().any(|part| {
                    !matches!(
                        part,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                });
            if output.pattern.trim().is_empty() || escapes {
                return Err(invalid(format!(
                    "output {name:?} pattern {:?} must be a path inside the job directory",
                    output.pattern
                )));
            }
        }
        Ok(())
    }

    /// The manifest of the job that installs this extension, or removes it.
    ///
    /// A job like any other - queued, logged, cancellable, run on the provider it is for -
    /// rather than a second way of running things: the same manifest with its install
    /// commands where its run commands were, and nothing to take in or bring back.
    pub fn maintenance(&self, action: Maintenance) -> Option<Manifest> {
        let install = self.install.as_ref()?;
        let commands: Vec<String> = match action {
            Maintenance::Install => install.steps.clone(),
            Maintenance::Remove if install.uninstall.is_empty() => {
                vec!["rm -rf {{ext.dir}}".to_string()]
            }
            Maintenance::Remove => install.uninstall.clone(),
        };
        let mut manifest = self.clone();
        manifest.name = format!("{}{}", action.prefix(), self.name);
        manifest.inputs.clear();
        manifest.outputs.clear();
        manifest.http = None;
        manifest.steps = commands
            .into_iter()
            .map(|command| RunStep {
                kind: StepKind::Shell,
                command,
                env: BTreeMap::new(),
                cwd: None,
                label: None,
            })
            .collect();
        Some(manifest)
    }

    /// True when the user must point us at weights before this model can run.
    pub fn needs_manual_weights(&self) -> bool {
        self.requirements.weights == WeightsPolicy::Manual
    }
}

/// Letters, digits, `-` and `_`: safe as a file name, a directory name, and unquoted in a
/// shell word.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
schema = 1
id = "demo"
name = "Demo"
task = "custom"

[[run]]
command = "echo hi"
"#;

    fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let dir = std::env::temp_dir().join(format!("geis-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.toml");
        std::fs::write(&path, text).unwrap();
        let result = Manifest::load(&path);
        std::fs::remove_dir_all(&dir).ok();
        result
    }

    #[test]
    fn a_minimal_manifest_loads_with_its_defaults() {
        let manifest = parse(MINIMAL).unwrap();
        assert_eq!(manifest.kind, Kind::Model);
        assert_eq!(manifest.requirements.gpu, GpuNeed::Recommended);
        assert_eq!(manifest.steps[0].kind, StepKind::Shell);
        assert!(manifest.source_path.is_some());
    }

    #[test]
    fn missing_or_mistyped_fields_are_errors_not_panics() {
        let missing_id = MINIMAL.replace("id = \"demo\"\n", "");
        assert!(matches!(
            parse(&missing_id),
            Err(ManifestError::Parse { .. })
        ));
        let wrong_type = MINIMAL.replace("schema = 1", "schema = \"one\"");
        assert!(matches!(
            parse(&wrong_type),
            Err(ManifestError::Parse { .. })
        ));
        let bad_task = MINIMAL.replace("\"custom\"", "\"juggling\"");
        assert!(matches!(parse(&bad_task), Err(ManifestError::Parse { .. })));
        let negative = MINIMAL.replace("schema = 1", "schema = -1");
        assert!(matches!(parse(&negative), Err(ManifestError::Parse { .. })));
        assert!(matches!(
            parse("not = [toml"),
            Err(ManifestError::Parse { .. })
        ));
        assert!(matches!(parse(""), Err(ManifestError::Parse { .. })));
        // A field this build does not know is not a reason to refuse the file.
        let extra = format!("{MINIMAL}\nsomething_new = 3\n");
        assert!(parse(&extra).is_ok());
        let schema_two = MINIMAL.replace("schema = 1", "schema = 2");
        assert!(matches!(
            parse(&schema_two),
            Err(ManifestError::Invalid { .. })
        ));
    }

    #[test]
    fn an_id_is_safe_as_a_directory_name() {
        for bad in ["", "../x", "a b", "a/b", "a.b", "$(id)"] {
            let text = MINIMAL.replace("id = \"demo\"", &format!("id = {bad:?}"));
            assert!(
                matches!(parse(&text), Err(ManifestError::Invalid { .. })),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn an_input_name_cannot_reach_outside_the_job() {
        // Input names become staged file names; this one would be written two levels up.
        let text = format!("{MINIMAL}\n[inputs]\n\"../../escape\" = {{ type = \"fasta\" }}\n");
        assert!(matches!(parse(&text), Err(ManifestError::Invalid { .. })));
        let dotted = format!("{MINIMAL}\n[inputs]\n\"seq.path\" = {{ type = \"text\" }}\n");
        assert!(matches!(parse(&dotted), Err(ManifestError::Invalid { .. })));
        let fine = format!("{MINIMAL}\n[inputs]\nseq_1 = {{ type = \"text\" }}\n");
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn an_output_pattern_stays_inside_the_job() {
        for bad in ["/etc/*", "../*.cif", "out/../../*.cif", ""] {
            let text =
                format!("{MINIMAL}\n[outputs]\nx = {{ type = \"any\", pattern = {bad:?} }}\n");
            assert!(
                matches!(parse(&text), Err(ManifestError::Invalid { .. })),
                "{bad:?} should be refused"
            );
        }
        let text = format!(
            "{MINIMAL}\n[outputs]\nx = {{ type = \"structure\", pattern = \"**/predictions/**/*.cif\" }}\n"
        );
        assert!(parse(&text).is_ok());
    }

    #[test]
    fn incomplete_sections_say_what_is_missing() {
        let no_steps = MINIMAL.replace("[[run]]\ncommand = \"echo hi\"\n", "");
        assert!(matches!(
            parse(&no_steps),
            Err(ManifestError::Invalid { .. })
        ));
        let enum_without_choices = format!("{MINIMAL}\n[inputs]\nmode = {{ type = \"enum\" }}\n");
        assert!(matches!(
            parse(&enum_without_choices),
            Err(ManifestError::Invalid { .. })
        ));
        let extension = MINIMAL.replace(
            "task = \"custom\"",
            "task = \"custom\"\nkind = \"extension\"",
        );
        assert!(matches!(
            parse(&extension),
            Err(ManifestError::Invalid { .. })
        ));
        let http_without_section = MINIMAL.replace(
            "task = \"custom\"",
            "task = \"custom\"\n[requirements]\nruntime = \"http\"",
        );
        assert!(matches!(
            parse(&http_without_section),
            Err(ManifestError::Invalid { .. })
        ));
        assert!(matches!(
            Manifest::load(Path::new("/definitely/not/here.toml")),
            Err(ManifestError::Io { .. })
        ));
    }

    #[test]
    fn removing_an_extension_without_its_own_steps_deletes_its_directory() {
        let text = MINIMAL.replace(
            "task = \"custom\"",
            "task = \"custom\"\nkind = \"extension\"",
        ) + "\n[install]\nsteps = [\"pip install x\"]\n";
        let manifest = parse(&text).unwrap();
        let remove = manifest.maintenance(Maintenance::Remove).unwrap();
        assert_eq!(remove.steps.len(), 1);
        assert_eq!(remove.steps[0].command, "rm -rf {{ext.dir}}");
        assert!(remove.name.starts_with(Maintenance::Remove.prefix()));
        let install = manifest.maintenance(Maintenance::Install).unwrap();
        assert_eq!(install.steps[0].command, "pip install x");
        assert!(parse(MINIMAL)
            .unwrap()
            .maintenance(Maintenance::Install)
            .is_none());
    }
}
