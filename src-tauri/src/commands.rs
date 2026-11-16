//! The IPC surface. Thin on purpose: validate, delegate, translate errors into strings
//! the UI can show. Logic belongs in `compute` and `store`, not here.

use crate::compute::job::{self, JobRequest, JobState};
use crate::compute::provider::{ProviderConfig, ProviderRecord, ProviderStatus};
use crate::compute::{HostProbe, ProviderKind};
use crate::models::manifest::Manifest;
use crate::store::{JobRecord, StoredArtifact};
use crate::{util, AppState, SECRET_SERVICE};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

/// Errors reach the UI as plain strings. Every message here is written to be shown to a
/// user directly - a scientist debugging their own cluster setup, not a developer reading
/// a stack trace.
type Reply<T> = Result<T, String>;

fn describe(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub data_dir: PathBuf,
    pub jobs_dir: PathBuf,
    pub user_models_dir: PathBuf,
    pub bundled_models_dir: Option<PathBuf>,
    pub model_count: usize,
    pub manifest_problems: Vec<crate::models::registry::LoadProblem>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    let registry = state.registry.read().unwrap();
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.paths.data_dir.clone(),
        jobs_dir: state.paths.jobs_dir.clone(),
        user_models_dir: state.paths.user_models_dir.clone(),
        bundled_models_dir: state.paths.bundled_models_dir.clone(),
        model_count: registry.len(),
        manifest_problems: registry.problems().to_vec(),
    }
}

// --- providers --------------------------------------------------------------

#[tauri::command]
pub fn list_providers(state: State<'_, AppState>) -> Reply<Vec<ProviderRecord>> {
    state.store.providers().map_err(describe)
}

#[derive(Debug, Deserialize)]
pub struct ProviderDraft {
    /// Absent for a new provider.
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub config: ProviderConfig,
}

fn yes() -> bool {
    true
}

#[tauri::command]
pub fn save_provider(state: State<'_, AppState>, draft: ProviderDraft) -> Reply<ProviderRecord> {
    if draft.name.trim().is_empty() {
        return Err("give this provider a name".into());
    }
    let config = draft.config.tidied()?;
    // An edit keeps the id so its jobs stay attached to it; a new provider gets a slug of
    // its name, uniquified.
    let (id, created_at, probe, status) = match &draft.id {
        Some(id) => {
            let existing = state.store.provider(id).map_err(describe)?;
            (
                existing.id,
                existing.created_at,
                existing.probe,
                // Configuration changed, so the cached probe describes the old target.
                ProviderStatus::Unknown,
            )
        }
        None => {
            let base = util::slugify(&draft.name);
            let taken = state.store.providers().map_err(describe)?;
            let mut id = base.clone();
            let mut suffix = 2;
            while taken.iter().any(|p| p.id == id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
            (id, util::now_iso(), None, ProviderStatus::Unknown)
        }
    };

    let record = ProviderRecord {
        id,
        name: draft.name.trim().to_string(),
        config,
        enabled: draft.enabled,
        created_at,
        probe,
        status,
    };
    state.store.upsert_provider(&record).map_err(describe)?;
    Ok(record)
}

#[tauri::command]
pub fn delete_provider(state: State<'_, AppState>, id: String) -> Reply<()> {
    // Jobs keep the provider id as history even after the provider is gone; they are not
    // cascaded away, because deleting a cluster config should not erase what ran on it.
    state.store.delete_provider(&id).map_err(describe)
}

#[tauri::command]
pub async fn probe_provider(state: State<'_, AppState>, id: String) -> Reply<HostProbe> {
    let queue = state.queue.clone();
    queue.probe_provider(&id).await.map_err(describe)
}

// --- models -----------------------------------------------------------------

/// A manifest plus everything the UI needs to decide whether to let someone run it.
#[derive(Debug, Serialize)]
pub struct ModelCard {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// Providers that could run this model at all, by kind. An HTTP-only model cannot run
    /// locally; a shell model cannot run on an HTTP endpoint.
    pub eligible_providers: Vec<String>,
    /// Set when the user has to supply gated weights before this will work.
    pub blocked_reason: Option<String>,
}

#[tauri::command]
pub fn list_models(state: State<'_, AppState>) -> Reply<Vec<ModelCard>> {
    let registry = state.registry.read().unwrap();
    let providers = state.store.providers().map_err(describe)?;
    let cards = registry
        .all()
        .map(|manifest| {
            let wants_http = manifest.requirements.runtime == crate::models::RuntimeKind::Http;
            let eligible: Vec<String> = providers
                .iter()
                .filter(|provider| provider.enabled)
                .filter(|provider| (provider.config.kind() == ProviderKind::Http) == wants_http)
                .map(|provider| provider.id.clone())
                .collect();

            let blocked_reason = if manifest.needs_manual_weights() {
                let has_weights = providers.iter().any(|provider| {
                    provider
                        .config
                        .exec_options()
                        .and_then(|options| options.weights_dir.as_ref())
                        .is_some()
                });
                (!has_weights).then(|| {
                    format!(
                        "{} needs weights you obtain and accept terms for yourself. \
                         Set a weights directory on a provider first.",
                        manifest.name
                    )
                })
            } else {
                None
            };

            ModelCard {
                manifest: manifest.clone(),
                eligible_providers: eligible,
                blocked_reason,
            }
        })
        .collect();
    Ok(cards)
}

#[tauri::command]
pub fn reload_models(state: State<'_, AppState>) -> usize {
    state.reload_models()
}

// --- jobs -------------------------------------------------------------------

#[tauri::command]
pub fn submit_job(state: State<'_, AppState>, request: JobRequest) -> Reply<JobRecord> {
    let manifest = {
        let registry = state.registry.read().unwrap();
        registry
            .get(&request.model_id)
            .cloned()
            .ok_or_else(|| format!("no model with id {:?}", request.model_id))?
    };
    let spec = job::plan(&manifest, &request, &state.paths.jobs_dir).map_err(describe)?;
    state.queue.submit(spec).map_err(describe)
}

/// Where an extension stands on one provider, as far as this app's own record goes.
#[derive(Debug, Serialize)]
pub struct ExtensionState {
    pub provider_id: String,
    /// `installed`, `installing`, `removing`, `failed` or `absent`.
    pub state: String,
    /// The install or removal job that says so, for its log.
    pub job_id: Option<String>,
    pub at: Option<String>,
}

/// Whether an extension is installed on each provider that could run it.
///
/// Read off the job history: the last install or removal run there, and how it ended. It
/// is this app's record of what it did, not an inspection of the host - a directory
/// deleted by hand is still "installed" here until it is reinstalled or removed.
#[tauri::command]
pub fn extension_states(state: State<'_, AppState>, id: String) -> Reply<Vec<ExtensionState>> {
    use crate::models::Maintenance;
    let providers = state.store.providers().map_err(describe)?;
    let jobs = state.store.jobs(None, 2000).map_err(describe)?;
    Ok(providers
        .iter()
        .filter(|provider| provider.enabled && provider.config.kind() != ProviderKind::Http)
        .map(|provider| {
            // Newest first, which is the order the store lists them in.
            let last = jobs.iter().find(|job| {
                job.model_id == id
                    && job.provider_id == provider.id
                    && (job.name.starts_with(Maintenance::Install.prefix())
                        || job.name.starts_with(Maintenance::Remove.prefix()))
                    && job.state != JobState::Cancelled
            });
            let state = match last {
                None => "absent",
                Some(job) => {
                    let installing = job.name.starts_with(Maintenance::Install.prefix());
                    match (installing, job.state) {
                        (true, JobState::Succeeded) => "installed",
                        (false, JobState::Succeeded) => "absent",
                        (_, JobState::Failed) => "failed",
                        (true, _) => "installing",
                        (false, _) => "removing",
                    }
                }
            };
            ExtensionState {
                provider_id: provider.id.clone(),
                state: state.to_string(),
                job_id: last.map(|job| job.id.clone()),
                at: last.map(|job| job.created_at.clone()),
            }
        })
        .collect())
}

/// Installs an extension on a provider, or removes it: a job, queued like any other.
#[tauri::command]
pub fn extension_maintain(
    state: State<'_, AppState>,
    id: String,
    provider_id: String,
    action: crate::models::Maintenance,
) -> Reply<JobRecord> {
    let manifest = {
        let registry = state.registry.read().unwrap();
        registry
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("no extension with id {id:?}"))?
    };
    let job_manifest = manifest.maintenance(action).ok_or_else(|| {
        format!(
            "{} is not something that is installed - it has no [install] section",
            manifest.name
        )
    })?;
    let request = JobRequest {
        model_id: id,
        provider_id,
        world_id: None,
        name: Some(job_manifest.name.clone()),
        inputs: Default::default(),
    };
    let spec = job::plan(&job_manifest, &request, &state.paths.jobs_dir).map_err(describe)?;
    state.queue.submit(spec).map_err(describe)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, id: String) -> Reply<bool> {
    Ok(state.queue.cancel(&id))
}

#[tauri::command]
pub fn list_jobs(
    state: State<'_, AppState>,
    world_id: Option<String>,
    limit: Option<u32>,
) -> Reply<Vec<JobRecord>> {
    state
        .store
        .jobs(world_id.as_deref(), limit.unwrap_or(100))
        .map_err(describe)
}

#[derive(Debug, Serialize)]
pub struct JobDetail {
    pub job: JobRecord,
    pub artifacts: Vec<StoredArtifact>,
    pub is_running: bool,
}

#[tauri::command]
pub fn job_detail(state: State<'_, AppState>, id: String) -> Reply<JobDetail> {
    let job = state.store.job(&id).map_err(describe)?;
    let artifacts = state.store.artifacts(&id).map_err(describe)?;
    Ok(JobDetail {
        is_running: state.queue.running_ids().contains(&id),
        job,
        artifacts,
    })
}

#[derive(Debug, Serialize)]
pub struct LogSlice {
    pub text: String,
    /// Byte offset to pass as `from` next time. Lets the terminal panel reattach to a
    /// running job without re-reading a log that may be hundreds of megabytes.
    pub next_offset: u64,
}

#[tauri::command]
pub fn read_job_log(state: State<'_, AppState>, id: String, from: Option<u64>) -> Reply<LogSlice> {
    let job = state.store.job(&id).map_err(describe)?;
    read_log_slice(&job.spec.log_path(), from)
}

/// Opening a finished job should not stall on a huge log; with no offset, the tail is
/// shown, which is where the failure is.
const LOG_TAIL: u64 = 512 * 1024;
/// The most one call reads from an offset. The rest comes on the next call, from where
/// this one stopped, rather than a log of hundreds of megabytes in one reply.
const LOG_READ_LIMIT: u64 = 8 * 1024 * 1024;

/// A piece of a job log, cut only between characters.
///
/// A log is written in whatever pieces the tool flushed, and a read that stops - or a tail
/// that starts - in the middle of a character used to turn it into replacement marks, for
/// good: the next read began after the bytes already sent. So the piece ends before a
/// character the job is still writing, and a tail starts at the next whole one.
fn read_log_slice(path: &std::path::Path, from: Option<u64>) -> Reply<LogSlice> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        // A queued job has no log yet; that is not an error worth showing.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogSlice {
                text: String::new(),
                next_offset: 0,
            })
        }
        Err(error) => return Err(describe(error)),
    };

    let length = file.metadata().map_err(describe)?.len();
    let (mut start, aligned) = match from {
        Some(from) if from <= length => (from, true),
        Some(_) => (0, true), // log was truncated or replaced under us
        None => (length.saturating_sub(LOG_TAIL), length <= LOG_TAIL),
    };
    file.seek(SeekFrom::Start(start)).map_err(describe)?;
    let mut buffer = Vec::new();
    file.take(LOG_READ_LIMIT)
        .read_to_end(&mut buffer)
        .map_err(describe)?;

    // A tail cut out of the middle of the file may begin inside a character.
    if !aligned {
        let skip = buffer
            .iter()
            .take(3)
            .take_while(|byte| *byte & 0b1100_0000 == 0b1000_0000)
            .count();
        buffer.drain(..skip);
        start += skip as u64;
    }
    let complete = util::utf8_complete_len(&buffer);
    buffer.truncate(complete);

    Ok(LogSlice {
        text: String::from_utf8_lossy(&buffer).into_owned(),
        // Where the read actually ended, not where the file ended before it: a job
        // writing between the two would have those bytes sent again on the next call.
        next_offset: start + buffer.len() as u64,
    })
}

// --- secrets ----------------------------------------------------------------

#[tauri::command]
pub fn set_secret(reference: String, value: String) -> Reply<()> {
    let entry = keyring::Entry::new(SECRET_SERVICE, &reference).map_err(describe)?;
    entry.set_password(&value).map_err(describe)
}

#[tauri::command]
pub fn has_secret(reference: String) -> bool {
    crate::legacy::read_secret(SECRET_SERVICE, &reference).is_some()
}

#[tauri::command]
pub fn delete_secret(reference: String) -> Reply<()> {
    // Under both names: a credential left under the old one would still be read.
    for service in [SECRET_SERVICE, crate::legacy::OLD_SECRET_SERVICE] {
        let entry = keyring::Entry::new(service, &reference).map_err(describe)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(describe(error)),
        }
    }
    Ok(())
}

// --- structures -------------------------------------------------------------

/// How many parsed structures stay resident; past that the least recently used is dropped.
///
/// It has to hold everything one scene draws, and the largest scene is the worlds shelf:
/// every world's molecules at once. At six - the first value - a seventh molecule pushed
/// the first out between evaluating a graph and building its scene, and the build failed
/// with "no longer open". A file already open is found again rather than parsed again, so
/// the count is of distinct files, not of requests.
const STRUCTURE_CACHE: usize = 64;

/// What the UI needs to know about an open structure. Deliberately without geometry or
/// coordinates: those cross the boundary once, as a scene, when something asks to draw.
#[derive(Debug, Clone, Serialize)]
pub struct StructureInfo {
    pub id: String,
    /// The PDB id for an archive entry, the data block name otherwise.
    pub entry_id: String,
    pub title: String,
    pub source: String,
    pub path: Option<PathBuf>,
    pub atom_count: u32,
    pub bond_count: u32,
    pub residue_count: u32,
    pub chains: Vec<ChainInfo>,
    pub b_factor_min: f32,
    pub b_factor_max: f32,
    /// True when the B-factor column holds pLDDT, so the UI can name the field correctly.
    pub looks_like_plddt: bool,
    /// Something about how it was opened that is worth one line on screen: that the
    /// entry's biological assembly was too large and the deposited unit is shown instead.
    pub note: Option<String>,
    /// The assemblies, models and alternate locations the file offers, and which of them
    /// this is - what the scene list lets the molecule be switched between.
    pub versions: crate::structure::model::Versions,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChainInfo {
    /// `asym_id_oper_id`, e.g. `A_1` - see `structure::Chain::id`.
    pub id: String,
    /// The author chain id, the letter in the paper.
    pub auth_id: String,
    /// `polymer`, `non-polymer`, `water` or `branched`.
    pub entity_type: String,
    pub residue_count: u32,
}

fn describe_structure(
    id: String,
    path: Option<PathBuf>,
    structure: &crate::structure::Structure,
) -> StructureInfo {
    let (min, max) = crate::geometry::color::b_factor_range(structure);
    StructureInfo {
        id,
        entry_id: structure.entry_id.clone(),
        title: structure.title.clone(),
        source: structure.source.clone(),
        path,
        atom_count: structure.atom_count() as u32,
        bond_count: structure.bonds.len() as u32,
        residue_count: structure.residues.len() as u32,
        chains: structure
            .chains
            .iter()
            .map(|chain| ChainInfo {
                id: chain.id.clone(),
                auth_id: chain.auth_id.clone(),
                entity_type: chain.entity_type.clone(),
                residue_count: chain.residue_count,
            })
            .collect(),
        b_factor_min: min,
        b_factor_max: max,
        looks_like_plddt: crate::geometry::color::looks_like_plddt(structure),
        note: None,
        versions: structure.versions.clone(),
    }
}

/// A file's identity for the cache: where it is and when it was last written. A file
/// rewritten in place - a prediction run again into the same directory - is a new file.
fn file_stamp(path: &std::path::Path) -> (PathBuf, Option<std::time::SystemTime>) {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok();
    (path.to_path_buf(), modified)
}

/// The id of this file if it is already open, marking it as just used.
fn recall(state: &AppState, stamp: &(PathBuf, Option<std::time::SystemTime>)) -> Option<String> {
    // No modification time means no way to tell a rewrite, so it is read again.
    stamp.1?;
    let mut cache = state.structures.lock().unwrap();
    let at = cache
        .iter()
        .position(|open| open.source.as_ref() == Some(stamp))?;
    let open = cache.remove(at);
    let id = open.id.clone();
    cache.push(open);
    Some(id)
}

fn remember(
    state: &AppState,
    structure: crate::structure::Structure,
    source: Option<(PathBuf, Option<std::time::SystemTime>)>,
) -> String {
    let id = util::new_id();
    let mut cache = state.structures.lock().unwrap();
    cache.push(crate::OpenStructure {
        id: id.clone(),
        structure: std::sync::Arc::new(structure),
        source,
    });
    while cache.len() > STRUCTURE_CACHE {
        cache.remove(0);
    }
    id
}

fn lookup(
    state: &AppState,
    id: &str,
) -> Result<std::sync::Arc<crate::structure::Structure>, String> {
    let mut cache = state.structures.lock().unwrap();
    let at = cache
        .iter()
        .position(|open| open.id == id)
        .ok_or_else(|| format!("structure {id} is no longer open"))?;
    // Used just now, so it goes to the back of the queue for eviction.
    let open = cache.remove(at);
    let structure = std::sync::Arc::clone(&open.structure);
    cache.push(open);
    Ok(structure)
}

/// Which version of an entry to read, as the window asks for it: an assembly by id, a
/// model by number, an alternate-location label. Each unsaid is the default.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReadingRequest {
    #[serde(default)]
    pub assembly_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub alternate: Option<String>,
}

impl From<&crate::graph::model::Version> for ReadingRequest {
    fn from(version: &crate::graph::model::Version) -> Self {
        ReadingRequest {
            assembly_id: version.assembly_id.clone(),
            model: version.model.clone(),
            alternate: version.alternate.clone(),
        }
    }
}

/// Parses a structure file and keeps it open.
#[tauri::command]
pub async fn open_structure(
    state: State<'_, AppState>,
    path: PathBuf,
    assembly: Option<bool>,
    reading: Option<ReadingRequest>,
) -> Reply<StructureInfo> {
    let reading = reading_of(assembly.unwrap_or(false), reading.unwrap_or_default());
    let id = open_as(&state, &path, reading, None).await?;
    let structure = lookup(&state, &id)?;
    Ok(describe_structure(id, Some(path), &structure))
}

fn reading_of(assembly: bool, request: ReadingRequest) -> crate::structure::Reading {
    use crate::structure::Unit;
    let unit = match (
        assembly,
        request.assembly_id.filter(|id| !id.trim().is_empty()),
    ) {
        (false, _) => Unit::Deposited,
        (true, Some(id)) => Unit::AssemblyId(id.trim().to_string()),
        (true, None) => Unit::Assembly,
    };
    crate::structure::Reading {
        unit,
        model: request
            .model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty()),
        alternate: request
            .alternate
            .and_then(|label| label.trim().chars().next()),
    }
}

/// Opens a file as the version asked for, or finds it already open as that.
///
/// The same file read two ways is two structures, so the cache keeps them apart. Read off
/// the main thread: a command that is not `async` runs on it, and a capsid's assembly is
/// seconds of work during which the window could not even draw the line that says so.
async fn open_as(
    state: &AppState,
    path: &std::path::Path,
    reading: crate::structure::Reading,
    entry: Option<&str>,
) -> Reply<String> {
    let stamp = unit_stamp(path, &reading);
    if let Some(id) = recall(state, &stamp) {
        return Ok(id);
    }
    let file = path.to_path_buf();
    let mut structure = tauri::async_runtime::spawn_blocking(move || {
        crate::structure::load_file_reading(&file, &reading)
    })
    .await
    .map_err(describe)?
    .map_err(describe)?;
    if let Some(entry) = entry {
        structure.entry_id = entry.to_ascii_uppercase();
    }
    Ok(remember(state, structure, Some(stamp)))
}

/// A file's identity in the cache as the version it was read as.
fn unit_stamp(
    path: &std::path::Path,
    reading: &crate::structure::Reading,
) -> (PathBuf, Option<std::time::SystemTime>) {
    use crate::structure::Unit;
    let (path, modified) = file_stamp(path);
    let mut name = path.into_os_string();
    match &reading.unit {
        Unit::Deposited => {}
        Unit::Assembly => name.push("#assembly"),
        Unit::AssemblyId(id) => name.push(format!("#assembly={id}")),
    }
    if let Some(model) = &reading.model {
        name.push(format!("#model={model}"));
    }
    if let Some(label) = reading.alternate {
        name.push(format!("#alt={label}"));
    }
    (PathBuf::from(name), modified)
}

/// Fetches a structure from the RCSB PDB by id, caching the file on disk, and opens it.
///
/// This is the one outbound request the app makes on its own behalf, and only when the
/// user types an id: the id goes to files.rcsb.org and nothing else leaves. The cache
/// means a second look at the same entry works offline.
#[tauri::command]
pub async fn fetch_structure(
    state: State<'_, AppState>,
    pdb_id: String,
    assembly: Option<bool>,
    reading: Option<ReadingRequest>,
) -> Reply<StructureInfo> {
    let id = pdb_id.trim().to_ascii_lowercase();
    if id.len() != 4 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!(
            // Said in full because the caller is often a model that reached for the wrong
            // tool, and the answer to "what should I have done" belongs in the error.
            "{pdb_id:?} is not a PDB id - they are four characters, like 1ubq. \
             A name like A or A_1 is a chain of a molecule that is already open; \
             to recolour one use colour_chain, and to restyle one use set_style."
        ));
    }

    let cache_dir = state.paths.data_dir.join("cache/rcsb");
    let cached = cache_dir.join(format!("{id}.cif"));
    if !cached.is_file() {
        let url = format!("https://files.rcsb.org/download/{id}.cif");
        let response = reqwest::get(&url)
            .await
            .map_err(|error| format!("could not reach files.rcsb.org: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(format!("the PDB has no entry {}", id.to_ascii_uppercase()));
        }
        if !response.status().is_success() {
            return Err(format!("files.rcsb.org returned {}", response.status()));
        }
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("download failed: {error}"))?;
        std::fs::create_dir_all(&cache_dir).map_err(describe)?;
        // Written beside its name and moved into place, so a download cut short is never
        // mistaken for the entry the next time it is asked for.
        let partial = cache_dir.join(format!("{id}.cif.part"));
        std::fs::write(&partial, &body).map_err(describe)?;
        std::fs::rename(&partial, &cached).map_err(describe)?;
    }

    // The entry's own file says how its biological assembly is made, so the assembly is
    // built from it here - StructCooker's way - rather than fetched as a second file. An
    // entry is its assembly unless the caller says the deposited coordinates are wanted.
    let reading = reading_of(assembly.unwrap_or(true), reading.unwrap_or_default());
    let handle = match open_as(&state, &cached, reading, Some(&id)).await {
        Ok(handle) => handle,
        Err(error) => {
            // A cached file that does not parse would fail the same way for ever;
            // dropping it lets the next request fetch the entry again.
            let _ = std::fs::remove_file(&cached);
            return Err(error);
        }
    };
    let structure = lookup(&state, &handle)?;
    Ok(describe_structure(handle, Some(cached), &structure))
}

/// One molecule in a scene request.
#[derive(Debug, Clone, Deserialize)]
pub struct ScenePart {
    pub structure_id: String,
    /// Which atoms this molecule includes.
    #[serde(default)]
    pub options: crate::geometry::SceneOptions,
    /// One style per chain, plus the unassigned one. Not a single representation, because
    /// a chain can be drawn differently from the rest.
    pub assignments: Vec<crate::geometry::Assignment>,
    /// The molecule's residue-level features, laid over its chain styles.
    #[serde(default)]
    pub features: Vec<crate::geometry::Feature>,
    /// Where this part should stand, for a viewport holding several worlds at once.
    #[serde(default)]
    pub place: Option<[f32; 3]>,
    /// The radius it is sized to when placed, so a row of them is framed alike.
    #[serde(default)]
    pub size: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SceneRequest {
    /// One per molecule. A viewport draws a scene, not a molecule.
    pub parts: Vec<ScenePart>,
    /// Background, palette and lighting. Colours depend on the background, so this is not
    /// separable from geometry.
    #[serde(default)]
    pub style: crate::geometry::SceneStyle,
    /// Build it for a preview rather than for study.
    ///
    /// The shelf of worlds asks for this: four ribbons an inch across, turning, where the
    /// full detail is tens of megabytes to build, hand over and draw - and indisting-
    /// uishable at that size.
    #[serde(default)]
    pub coarse: bool,
}

/// Builds geometry for a representation and returns it as a binary payload.
///
/// Geometry is built here, not in the renderer, because the Blender render path needs the
/// same answer. A cartoon defined twice - once in the viewport and once in a render script
/// - drifts, and the day it does, a world's picture stops matching the viewer it was made
/// from.
#[tauri::command]
pub async fn build_scene(
    state: State<'_, AppState>,
    request: SceneRequest,
) -> Reply<tauri::ipc::Response> {
    // Each molecule is built on its own and the results concatenated, so two structures
    // in one viewport keep their own styles.
    let mut scene = crate::geometry::Scene::empty();
    // A part can be told where to stand and how big to be, which is what lets the shelf of
    // worlds be one viewport with a row of them in it. Parts told to stand in the same
    // place are one world's molecules: they are held until the last of them is built and
    // then moved together, by the box around all of them.
    let mut held: Vec<crate::geometry::Scene> = Vec::new();
    let mut held_at: Option<([f32; 3], f32)> = None;
    let settle = |held: &mut Vec<crate::geometry::Scene>,
                  at: Option<([f32; 3], f32)>,
                  scene: &mut crate::geometry::Scene| {
        let drew: Vec<&crate::geometry::Scene> = held
            .iter()
            .filter(|part| part.drawn_atoms > 0 || !part.mesh.is_empty())
            .collect();
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for part in &drew {
            for axis in 0..3 {
                low[axis] = low[axis].min(part.bounds_min[axis]);
                high[axis] = high[axis].max(part.bounds_max[axis]);
            }
        }
        for mut part in held.drain(..) {
            if let (Some((at, size)), true) = (at, low[0].is_finite()) {
                part.place_within(low, high, at, size);
            }
            scene.append(part);
        }
    };
    // How many residues the whole scene holds, so that every part is drawn at the detail
    // the scene as a whole can afford.
    let mut opened = Vec::with_capacity(request.parts.len());
    for part in &request.parts {
        opened.push(lookup(&state, &part.structure_id)?);
    }
    let scene_residues: usize = opened
        .iter()
        .map(|structure| structure.residues.len())
        .sum();
    // Off the async runtime, and each molecule on its own core: the shelf asks for every
    // world's molecules in one scene, and they were built one after another on a thread
    // the rest of the app's commands were waiting for.
    let request = std::sync::Arc::new(request);
    let work = request.clone();
    let built: Vec<crate::geometry::Scene> = tauri::async_runtime::spawn_blocking(move || {
        let request = &*work;
        let opened = &opened;
        std::thread::scope(|scope| {
            let handles: Vec<_> = request
                .parts
                .iter()
                .zip(opened)
                .map(|(part, structure)| {
                    scope.spawn(move || {
                        crate::geometry::representation::build_scene_among(
                            structure,
                            part.options,
                            &part.assignments,
                            &part.features,
                            &request.style,
                            request.coarse,
                            scene_residues,
                        )
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| crate::geometry::Scene::empty())
                })
                .collect()
        })
    })
    .await
    .map_err(describe)?;
    for (part, built) in request.parts.iter().zip(built) {
        let at = part.place.map(|at| (at, part.size.unwrap_or(10.0)));
        if at != held_at {
            settle(&mut held, held_at, &mut scene);
            held_at = at;
        }
        held.push(built);
    }
    settle(&mut held, held_at, &mut scene);
    scene.settle();
    Ok(tauri::ipc::Response::new(scene.encode()))
}

/// Legend rows for a colour scheme, so the swatches beside the viewport are the same
/// colours the geometry was built with.
#[tauri::command]
pub fn scene_legend(
    state: State<'_, AppState>,
    structure_id: String,
    color_scheme: crate::geometry::ColorScheme,
    style: Option<crate::geometry::SceneStyle>,
) -> Reply<Vec<crate::geometry::LegendEntry>> {
    let structure = lookup(&state, &structure_id)?;
    let style = style.unwrap_or_default();
    Ok(crate::geometry::color::legend(
        &structure,
        color_scheme,
        &style.color_context(),
    ))
}

/// The colour swatches the picker offers - Open Color's ladders plus the categorical sets.
#[tauri::command]
pub fn color_swatches() -> Vec<crate::geometry::SwatchGroup> {
    crate::geometry::palette::swatches()
}

/// The default lighting rig and style, so the UI starts from what the renderer would use.
#[tauri::command]
pub fn default_style() -> crate::geometry::SceneStyle {
    crate::geometry::SceneStyle::default()
}

#[tauri::command]
pub fn close_structure(state: State<'_, AppState>, id: String) {
    state
        .structures
        .lock()
        .unwrap()
        .retain(|open| open.id != id);
}

/// Takes the structure path given on the command line, if there was one.
///
/// Taking rather than reading: it must open once, not again on every reload of the UI.
#[tauri::command]
pub fn pending_open(state: State<'_, AppState>) -> Option<PathBuf> {
    state
        .pending_open
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

// --- the node graph ---------------------------------------------------------

/// A node as the editor draws it.
///
/// Sockets come from the core rather than being mirrored in the editor: their names,
/// types and colours are what decide which connections are legal, and a second copy of
/// that table in TypeScript would drift from the one that enforces it.
#[derive(Debug, Serialize)]
pub struct NodeDescription {
    pub id: String,
    pub title: String,
    pub type_name: String,
    /// The family this node belongs to, and the colour its header is tinted with.
    pub category: String,
    pub accent: [u8; 3],
    pub inputs: Vec<SocketDescription>,
    pub output: Option<SocketDescription>,
    pub position: [f32; 2],
}

#[derive(Debug, Serialize)]
pub struct SocketDescription {
    pub name: String,
    pub kind: crate::graph::SocketKind,
    /// What to call the kind on screen; the tag itself is kebab-case for the wire.
    pub kind_label: String,
    pub color: [u8; 3],
    pub multiple: bool,
    pub required: bool,
}

fn describe_socket(socket: &crate::graph::Socket) -> SocketDescription {
    SocketDescription {
        name: socket.name.clone(),
        kind: socket.kind,
        kind_label: socket.kind.label().to_string(),
        color: socket.kind.color(),
        multiple: socket.multiple,
        required: socket.required,
    }
}

fn describe_nodes(graph: &crate::graph::Graph) -> Vec<NodeDescription> {
    graph
        .nodes
        .iter()
        .map(|node| {
            let (category, accent) = node.kind.category();
            NodeDescription {
                id: node.id.clone(),
                title: node.kind.title(),
                type_name: node.kind.type_name().to_string(),
                category: category.to_string(),
                accent,
                inputs: node.kind.inputs().iter().map(describe_socket).collect(),
                output: node.kind.output().map(|kind| SocketDescription {
                    name: "Out".into(),
                    kind,
                    kind_label: kind.label().to_string(),
                    color: kind.color(),
                    multiple: true,
                    required: false,
                }),
                position: node.position,
            }
        })
        .collect()
}

/// One entry in the editor's Add menu.
#[derive(Debug, Serialize)]
pub struct NodeTemplate {
    pub type_name: String,
    pub title: String,
    /// Which section of the Add menu it belongs in.
    pub group: String,
    /// A ready-to-insert node, minus its id and position.
    pub kind: crate::graph::NodeKind,
}

/// The graph a new session starts with.
#[tauri::command]
pub fn graph_load(state: State<'_, AppState>) -> crate::graph::Graph {
    // The graph belongs to the world that is open; the settings row below is what sessions
    // from before worlds existed left behind, and `all_worlds` moves it into the first one.
    let worlds = all_worlds(&state);
    let active = active_world_id(&state, &worlds);
    if let Some(world) = worlds.into_iter().find(|world| world.id == active) {
        return world.graph;
    }
    graph_load_raw(&state)
}

fn graph_load_raw(state: &AppState) -> crate::graph::Graph {
    // The graph the last session left behind, if there is one. Everything the app does is
    // in the graph - which molecule, how it is drawn, the lighting, the render settings,
    // which nodes were folded - so without this, closing the window threw all of it away
    // and reopened on the default pipeline.
    match state.store.setting(GRAPH_SETTING) {
        Ok(Some(saved)) => match serde_json::from_str::<crate::graph::Graph>(&saved) {
            Ok(graph) if !graph.nodes.is_empty() => return graph,
            Ok(_) => {}
            Err(error) => {
                // A graph written by an older build may no longer parse. Starting fresh
                // beats refusing to open, but it is worth saying out loud.
                eprintln!("[graph] the saved graph could not be read, starting fresh: {error}");
            }
        },
        Ok(None) => {}
        Err(error) => eprintln!("[graph] the saved graph could not be read: {error}"),
    }
    crate::graph::default_graph()
}

/// Where the working graph lives between sessions.
const GRAPH_SETTING: &str = "graph";
const LAYOUT_SETTING: &str = "layout";

/// The screen layout the last session left behind: which areas, split how, showing what.
///
/// Stored as the JSON the frontend produces and read back untouched; the shape belongs to
/// the frontend's `areas.ts`, and the core has no opinion on it beyond keeping it.
#[tauri::command]
pub fn layout_load(state: State<'_, AppState>) -> Option<String> {
    state.store.setting(LAYOUT_SETTING).ok().flatten()
}

#[tauri::command]
pub fn layout_save(state: State<'_, AppState>, layout: String) -> Reply<()> {
    state
        .store
        .set_setting(LAYOUT_SETTING, &layout)
        .map_err(describe)
}

/// Saves the working graph.
///
/// Called on every change rather than on quit: a crash or a force-quit is exactly when
/// losing an afternoon's pipeline hurts, and the whole graph is a few kilobytes of JSON.
#[tauri::command]
pub fn graph_save(state: State<'_, AppState>, graph: crate::graph::Graph) -> Reply<()> {
    // Into the world that is open, which is what a session is.
    let mut worlds = all_worlds(&state);
    let active = active_world_id(&state, &worlds);
    match worlds.iter_mut().find(|world| world.id == active) {
        Some(world) => {
            world.graph = graph;
            write_worlds(&state, &worlds)
        }
        None => {
            let encoded = serde_json::to_string(&graph).map_err(describe)?;
            state
                .store
                .set_setting(GRAPH_SETTING, &encoded)
                .map_err(describe)
        }
    }
}

/// Throws the saved graph away and returns the default one.
#[tauri::command]
pub fn graph_reset(state: State<'_, AppState>) -> Reply<crate::graph::Graph> {
    let fresh = crate::graph::default_graph();
    let mut worlds = all_worlds(&state);
    let active = active_world_id(&state, &worlds);
    if let Some(world) = worlds.iter_mut().find(|world| world.id == active) {
        world.graph = fresh.clone();
        write_worlds(&state, &worlds)?;
        return Ok(fresh);
    }
    graph_reset_raw(state)
}

fn graph_reset_raw(state: State<'_, AppState>) -> Reply<crate::graph::Graph> {
    state
        .store
        .set_setting(GRAPH_SETTING, "")
        .map_err(describe)?;
    Ok(crate::graph::default_graph())
}

/// One chain's sequence, as two strings the same length.
///
/// Strings rather than a list of residue objects: a ribosome has a hundred thousand
/// residues, and `[{"code":"A","secondary":0},...]` for each of them is megabytes of JSON
/// to say what two strings say.
#[derive(Debug, Serialize)]
pub struct ChainSequence {
    pub chain: String,
    /// One-letter codes, in residue order.
    pub codes: String,
    /// `H` for helix, `E` for strand, `-` for anything else. One character per code.
    pub secondary: String,
    /// Author number of the first residue, so the strip can label positions.
    pub first_number: i32,
    /// The index of each residue in the structure, one per code, so a drag across the
    /// strip can name exactly the residues it covered.
    pub residue_indices: Vec<u32>,
}

/// Which atoms and residues of a structure belong to a set of chains.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionMask {
    /// One byte per atom: 1 when the atom is in one of the chains.
    pub atoms: Vec<u8>,
    /// One byte per residue.
    pub residues: Vec<u8>,
}

/// What the viewport was pointing at: enough to name it, at every level.
#[derive(Debug, Clone, Serialize)]
pub struct PickTarget {
    /// The chain's id, `A_1`.
    pub chain: String,
    /// Its author id, for the label people read.
    pub auth_chain: String,
    pub residue_index: u32,
    /// Author residue number, which is what people cite.
    pub residue_number: i32,
    pub residue_name: String,
    /// The atom, when a sphere or a stick was hit rather than a ribbon.
    pub atom_index: Option<u32>,
    pub atom_name: Option<String>,
    pub element: Option<String>,
}

/// Resolves a picked id to the thing it belongs to.
///
/// The viewport can only report what its geometry knows - a sphere knows its atom, a
/// ribbon vertex knows its residue - and turning that into "chain B, His 92, NE2" needs
/// the structure, which lives here. Written to answer all three levels at once, because
/// the plan is to pick residues and atoms as well as chains and the difference is which
/// field the caller reads, not another round trip.
#[tauri::command]
pub fn pick_target(
    state: State<'_, AppState>,
    id: String,
    kind: String,
    index: u32,
) -> Reply<Option<PickTarget>> {
    let structure = lookup(&state, &id)?;

    let (residue_index, atom_index) = match kind.as_str() {
        "atom" => {
            let atom = index as usize;
            if atom >= structure.atom_count() {
                return Ok(None);
            }
            match structure.residue_of_atom.get(atom) {
                Some(residue) => (*residue, Some(index)),
                None => return Ok(None),
            }
        }
        "residue" => (index, None),
        _ => return Ok(None),
    };

    let Some(residue) = structure.residues.get(residue_index as usize) else {
        return Ok(None);
    };
    let (chain, auth_chain) = structure
        .chains
        .get(residue.chain_index as usize)
        .map(|chain| (chain.id.clone(), chain.auth_id.clone()))
        .unwrap_or_default();

    Ok(Some(PickTarget {
        chain,
        auth_chain,
        residue_index,
        residue_number: residue.number,
        residue_name: residue.name.clone(),
        atom_index,
        atom_name: atom_index
            .and_then(|atom| structure.atom_names.get(atom as usize))
            .map(|name| name.trim().to_string()),
        element: atom_index
            .and_then(|atom| structure.elements.get(atom as usize))
            .map(|element| crate::structure::chem::symbol(*element).to_string()),
    }))
}

// --- worlds ----------------------------------------------------------------------------
//
// A world is a session: the graph, and where the camera was left. The shelf of them is one
// JSON list in the settings table; only the card pictures are files. See `worlds`.

fn all_worlds(state: &AppState) -> Vec<crate::worlds::World> {
    let mut worlds: Vec<crate::worlds::World> =
        match state.store.list_setting(crate::worlds::SETTING) {
            Ok(worlds) => worlds,
            Err(error) => {
                // The database could not be asked. That is not the same as there being no
                // worlds, and writing a fresh shelf over one that merely failed to load
                // is the one thing that must not happen - so this one is not saved.
                eprintln!("[worlds] the shelf could not be read: {error}");
                return vec![crate::worlds::World::new(
                    "My world".into(),
                    crate::graph::default_graph(),
                )];
            }
        };
    if worlds.is_empty() {
        // A session from before worlds existed becomes the first one, rather than being
        // left in a settings row nothing reads any more.
        let graph = graph_load_raw(state);
        // On a first launch there is no such session, and the world made here is as new as
        // one made from the shelf: it opens laid out the way the preferences say a new
        // world is, not with every pane up.
        let first_launch = !matches!(state.store.setting(GRAPH_SETTING), Ok(Some(_)));
        let world = crate::worlds::World::new("My world".into(), graph);
        if first_launch {
            let _ = state
                .store
                .set_setting(&layout_key(&world.id), DEFAULT_LAYOUT);
        }
        worlds.push(world);
        let _ = write_worlds(state, &worlds);
    }
    worlds
}

fn write_worlds(state: &AppState, worlds: &[crate::worlds::World]) -> Result<(), String> {
    let text = serde_json::to_string(worlds).map_err(describe)?;
    state
        .store
        .set_setting(crate::worlds::SETTING, &text)
        .map_err(describe)
}

/// Which world is open. The first on the shelf when nothing says otherwise.
fn active_world_id(state: &AppState, worlds: &[crate::worlds::World]) -> String {
    let saved = state
        .store
        .setting(crate::worlds::ACTIVE)
        .ok()
        .flatten()
        .unwrap_or_default();
    if worlds.iter().any(|world| world.id == saved) {
        return saved;
    }
    worlds
        .first()
        .map(|world| world.id.clone())
        .unwrap_or_default()
}

/// The shelf: every world, without the graphs, oldest first.
///
/// In the order they were made, not the order they were last used. A shelf that sorts
/// itself by recency rearranges behind your back - you come out of a world to find it has
/// jumped to the front and its neighbours have moved - and the row is a place you learn,
/// like a rack of records. It also means leaving a world changes nothing about the row, so
/// the shelf does not have to build itself again to show the same thing.
#[tauri::command]
pub fn worlds_list(state: State<'_, AppState>) -> Vec<crate::worlds::WorldCard> {
    let mut worlds = all_worlds(&state);
    worlds.sort_by_key(|world| world.created);
    worlds.iter().map(crate::worlds::card).collect()
}

#[tauri::command]
pub fn world_active(state: State<'_, AppState>) -> String {
    let worlds = all_worlds(&state);
    active_world_id(&state, &worlds)
}

/// A new, empty world - the default pipeline and nothing open.
#[tauri::command]
pub fn world_create(state: State<'_, AppState>, name: String) -> Reply<crate::worlds::WorldCard> {
    let mut worlds = all_worlds(&state);
    let world = crate::worlds::World::new(
        if name.trim().is_empty() {
            "New world".into()
        } else {
            name.trim().to_string()
        },
        crate::graph::default_graph(),
    );
    let card = crate::worlds::card(&world);
    worlds.push(world);
    write_worlds(&state, &worlds)?;
    // A world made now starts on the default arrangement - the viewport and nothing else.
    // Said outright rather than left unset: a world from before layouts were per world has
    // none either, and that one should keep the arrangement it was last seen in.
    state
        .store
        .set_setting(&layout_key(&card.id), DEFAULT_LAYOUT)
        .map_err(describe)?;
    Ok(card)
}

/// The user's preferences, as the settings screen saved them: one JSON object, whose
/// fields the frontend owns - defaults for what is new, not state of what exists.
pub const PREFERENCES: &str = "preferences";

#[tauri::command]
pub fn preferences(state: State<'_, AppState>) -> Reply<serde_json::Value> {
    let saved = state.store.setting(PREFERENCES).map_err(describe)?;
    let preferences: serde_json::Value = saved
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    // Read at start, which is when the frontend first asks: the palette takes them up
    // before anything is drawn.
    apply_theme(&preferences);
    Ok(preferences)
}

#[tauri::command]
pub fn preferences_save(state: State<'_, AppState>, preferences: serde_json::Value) -> Reply<()> {
    if !preferences.is_object() {
        return Err("preferences are an object".into());
    }
    let text = serde_json::to_string(&preferences).map_err(describe)?;
    apply_theme(&preferences);
    state
        .store
        .set_setting(PREFERENCES, &text)
        .map_err(describe)
}

/// Hands the colour conventions the preferences name to the palette that draws with them.
pub fn apply_theme(preferences: &serde_json::Value) {
    crate::geometry::palette::set_theme(
        preferences.get("elements").and_then(|value| value.as_str()),
        preferences
            .get("secondary")
            .and_then(|value| value.as_str()),
    );
}

/// What a world's layout is saved as until it has been arranged.
const DEFAULT_LAYOUT: &str = "default";

fn layout_key(world: &str) -> String {
    format!("layout:{world}")
}

/// How a world's panes were left: `default` for one never arranged, nothing for a world
/// older than per-world layouts, otherwise what the screen saved.
#[tauri::command]
pub fn world_layout(state: State<'_, AppState>, id: String) -> Reply<Option<String>> {
    state.store.setting(&layout_key(&id)).map_err(describe)
}

/// Finds contacts of one kind in a structure, or around some of its residues: hydrogen
/// bonds, salt bridges, pi stacking, hydrophobic contacts.
#[tauri::command]
pub fn find_contacts(
    state: State<'_, AppState>,
    id: String,
    kind: crate::structure::contacts::ContactKind,
    residues: Option<Vec<u32>>,
    waters: Option<bool>,
) -> Reply<Vec<crate::structure::contacts::Contact>> {
    let structure = lookup(&state, &id)?;
    Ok(crate::structure::contacts::find(
        &structure,
        kind,
        residues.as_deref(),
        waters.unwrap_or(false),
    ))
}

/// The residues of a structure's hydrophobic core, by index.
#[tauri::command]
pub fn find_core(state: State<'_, AppState>, id: String) -> Reply<Vec<u32>> {
    let structure = lookup(&state, &id)?;
    Ok(crate::structure::contacts::hydrophobic_core(&structure))
}

/// One of the analyses every structure gets when it is opened.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Polar,
    SaltBridge,
    PiStacking,
    Hydrophobic,
    Core,
    Site,
    Interface,
    Ligands,
}

/// What a preset found: contacts between residues, or a set of residues.
#[derive(Debug, Clone)]
pub enum PresetResult {
    Contacts(Vec<crate::structure::contacts::Contact>),
    Residues(Vec<u32>),
}

impl PresetResult {
    /// Packed for the window as numbers, not as JSON.
    ///
    /// A capsid has half a million hydrogen bonds. As JSON, each with the names of its two
    /// atoms, that was 80 MB for one kind and 160 MB for two, parsed into a million
    /// objects in a window that was by then also holding the scene. Laid out flat it is
    /// 36 bytes a contact and nothing to parse. Little-endian throughout:
    ///
    /// - `u32` kind: 0 for residues, 1 for contacts; then `u32` count;
    /// - residues: `count` residue indices, `u32`;
    /// - contacts: `count` records of `u32` residue, `u32` residue, `f32` distance, then
    ///   `f32` x, y, z of one end and of the other.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let word = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
        let float = |out: &mut Vec<u8>, value: f32| out.extend_from_slice(&value.to_le_bytes());
        match self {
            PresetResult::Residues(residues) => {
                out.reserve(8 + residues.len() * 4);
                word(&mut out, 0);
                word(&mut out, residues.len() as u32);
                for &residue in residues {
                    word(&mut out, residue);
                }
            }
            PresetResult::Contacts(contacts) => {
                out.reserve(8 + contacts.len() * 36);
                word(&mut out, 1);
                word(&mut out, contacts.len() as u32);
                for contact in contacts {
                    word(&mut out, contact.residues[0]);
                    word(&mut out, contact.residues[1]);
                    float(&mut out, contact.distance);
                    for value in contact.from.iter().chain(&contact.to) {
                        float(&mut out, *value);
                    }
                }
            }
        }
        out
    }
}

/// Runs one preset analysis of a structure.
///
/// Off the main thread: on a capsid the buried-surface search is seconds of work, and a
/// command that is not `async` holds the window still for as long as it runs. Each preset
/// is its own call so that the quick ones are on screen while the slow one is still going.
#[tauri::command]
pub async fn structure_preset(
    state: State<'_, AppState>,
    id: String,
    preset: Preset,
) -> Reply<tauri::ipc::Response> {
    use crate::structure::contacts::{self, ContactKind};
    use crate::structure::model::ResidueKind;
    let structure = lookup(&state, &id)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Unnamed: the window is sent positions and counts, never the ends' names.
        let found = |kind| {
            PresetResult::Contacts(contacts::find_named(&structure, kind, None, false, false))
        };
        match preset {
            Preset::Polar => found(ContactKind::Polar),
            Preset::SaltBridge => found(ContactKind::SaltBridge),
            Preset::PiStacking => found(ContactKind::PiStacking),
            Preset::Hydrophobic => found(ContactKind::Hydrophobic),
            Preset::Core => PresetResult::Residues(contacts::hydrophobic_core(&structure)),
            Preset::Site => PresetResult::Residues(contacts::binding_site(&structure)),
            Preset::Interface => PresetResult::Residues(contacts::interface(&structure)),
            Preset::Ligands => PresetResult::Residues(
                structure
                    .residues
                    .iter()
                    .enumerate()
                    .filter(|(_, residue)| residue.kind == ResidueKind::Ligand)
                    .map(|(index, _)| index as u32)
                    .collect(),
            ),
        }
    })
    .await
    .map(|found| tauri::ipc::Response::new(found.encode()))
    .map_err(describe)
}

/// Where cleared conversations are kept until there are too many of them.
fn chat_bin(state: &AppState) -> PathBuf {
    state.paths.data_dir.join("tmp/chats")
}

/// The most a bin of cleared conversations may hold, in bytes, before the oldest go.
const CHAT_BIN_LIMIT: u64 = 20 * 1024 * 1024;

/// Puts a cleared conversation in the bin, and empties the oldest out past the limit.
///
/// Clearing is undone from here. A file each, named by when and for which world, so the
/// order they are emptied in is the order they were cleared in.
#[tauri::command]
pub fn chat_bin_put(state: State<'_, AppState>, id: String, chat: serde_json::Value) -> Reply<()> {
    let bytes = serde_json::to_vec(&chat).map_err(describe)?;
    put_in_bin(&chat_bin(&state), &id, &bytes, CHAT_BIN_LIMIT)
}

/// A world id is part of a file name in the bin, so it has to be one that names a file
/// there and nowhere else. World ids are uuids; anything else did not come from a world.
fn bin_safe(id: &str) -> Reply<&str> {
    if !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(id)
    } else {
        Err(format!("{id:?} is not a world id"))
    }
}

fn put_in_bin(bin: &std::path::Path, id: &str, bytes: &[u8], limit: u64) -> Reply<()> {
    let id = bin_safe(id)?;
    std::fs::create_dir_all(bin).map_err(describe)?;
    let name = format!("{:013}-{}.json", crate::worlds::now_ms(), id);
    std::fs::write(bin.join(&name), bytes).map_err(describe)?;

    let mut files: Vec<(String, u64)> = std::fs::read_dir(bin)
        .map_err(describe)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            Some((
                entry.file_name().to_string_lossy().to_string(),
                entry.metadata().ok()?.len(),
            ))
        })
        .collect();
    files.sort();
    let mut total: u64 = files.iter().map(|(_, size)| size).sum();
    // The newest is never emptied, however large: it is the one an undo would want. Found
    // by name rather than by position - two clears in one millisecond sort by world id,
    // and the one just written is not necessarily last.
    for (old, size) in &files {
        if total <= limit {
            break;
        }
        if *old == name {
            continue;
        }
        std::fs::remove_file(bin.join(old)).ok();
        total -= size;
    }
    Ok(())
}

/// Takes a world's most recently cleared conversation back out of the bin, if it has one.
#[tauri::command]
pub fn chat_bin_take(state: State<'_, AppState>, id: String) -> Reply<Option<serde_json::Value>> {
    take_from_bin(&chat_bin(&state), &id)
}

fn take_from_bin(bin: &std::path::Path, id: &str) -> Reply<Option<serde_json::Value>> {
    let id = bin_safe(id)?;
    let suffix = format!("-{id}.json");
    let mut names: Vec<String> = match std::fs::read_dir(bin) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(&suffix))
            .collect(),
        Err(_) => return Ok(None),
    };
    names.sort();
    let Some(newest) = names.pop() else {
        return Ok(None);
    };
    let path = bin.join(newest);
    let chat = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    std::fs::remove_file(&path).ok();
    Ok(chat)
}

/// A skill: a named set of instructions the user can call on from the agent's box.
#[derive(Debug, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// The user's own skills: every `.md` file in the app's `skills` directory.
///
/// The file's name is the skill's; its first line, if it starts with `#`, says what it is
/// for; the rest is what the agent is told. A folder of text files, because a skill is
/// something people write, keep and hand to each other.
#[tauri::command]
pub fn agent_skills(state: State<'_, AppState>) -> Reply<Vec<Skill>> {
    let directory = state.paths.data_dir.join("skills");
    std::fs::create_dir_all(&directory).ok();
    let mut skills = Vec::new();
    for entry in std::fs::read_dir(&directory)
        .map_err(describe)?
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let (Some(name), Ok(text)) = (
            path.file_stem().and_then(|stem| stem.to_str()),
            std::fs::read_to_string(&path),
        ) else {
            continue;
        };
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("");
        let (description, body) = match first.strip_prefix('#') {
            Some(title) => (
                title.trim().to_string(),
                lines.collect::<Vec<_>>().join("\n"),
            ),
            None => (String::new(), text.clone()),
        };
        skills.push(Skill {
            name: name.to_lowercase().replace(' ', "-"),
            description,
            body: body.trim().to_string(),
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

fn view_key(world: &str) -> String {
    format!("view:{world}")
}

/// A world's viewport data that is not the graph's: its saved cameras, and which is in
/// use. One JSON object the screen owns; empty for a world that has none.
#[tauri::command]
pub fn world_view(state: State<'_, AppState>, id: String) -> Reply<serde_json::Value> {
    let saved = state.store.setting(&view_key(&id)).map_err(describe)?;
    Ok(saved
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({})))
}

#[tauri::command]
pub fn world_view_save(
    state: State<'_, AppState>,
    id: String,
    view: serde_json::Value,
) -> Reply<()> {
    if !all_worlds(&state).iter().any(|world| world.id == id) {
        return Err(format!("no world called {id}"));
    }
    let text = serde_json::to_string(&view).map_err(describe)?;
    state
        .store
        .set_setting(&view_key(&id), &text)
        .map_err(describe)
}

#[tauri::command]
pub fn world_layout_save(state: State<'_, AppState>, id: String, layout: String) -> Reply<()> {
    if !all_worlds(&state).iter().any(|world| world.id == id) {
        return Err(format!("no world called {id}"));
    }
    state
        .store
        .set_setting(&layout_key(&id), &layout)
        .map_err(describe)
}

#[tauri::command]
pub fn world_rename(state: State<'_, AppState>, id: String, name: String) -> Reply<()> {
    let mut worlds = all_worlds(&state);
    let world = worlds
        .iter_mut()
        .find(|world| world.id == id)
        .ok_or_else(|| format!("no world called {id}"))?;
    world.name = if name.trim().is_empty() {
        world.name.clone()
    } else {
        name.trim().to_string()
    };
    write_worlds(&state, &worlds)
}

/// Removes a world and its picture. The last one cannot go: a shelf with nothing on it has
/// no way back into the app.
#[tauri::command]
pub fn world_delete(state: State<'_, AppState>, id: String) -> Reply<()> {
    let mut worlds = all_worlds(&state);
    if worlds.len() <= 1 {
        return Err("that is the only world there is".into());
    }
    worlds.retain(|world| world.id != id);
    write_worlds(&state, &worlds)?;
    if let Ok(path) = thumbnail_path(&state, &id) {
        let _ = std::fs::remove_file(path);
    }
    forget_world(&state.store, &id, &worlds)
}

/// Where a world's picture for the shelf is kept.
fn thumbnail_path(state: &AppState, id: &str) -> Reply<PathBuf> {
    Ok(state
        .paths
        .data_dir
        .join("thumbnails")
        .join(format!("{}.png", bin_safe(id)?)))
}

/// Keeps a picture of a world, taken as it is left, for the shelf to show at once.
///
/// The shelf builds every world in 3D, and a world holding a capsid takes seconds to
/// build; the picture is what stands in its place until it has been.
#[tauri::command]
pub fn world_thumbnail_save(state: State<'_, AppState>, id: String, data_url: String) -> Reply<()> {
    let path = thumbnail_path(&state, &id)?;
    save_image(path, data_url).map(|_| ())
}

/// A world's picture, if one was taken: where it is, and when it was written, so a
/// picture taken again is fetched again rather than shown from the cache.
#[derive(Debug, Serialize)]
pub struct Thumbnail {
    pub path: PathBuf,
    pub written: u64,
}

#[tauri::command]
pub fn world_thumbnail(state: State<'_, AppState>, id: String) -> Reply<Option<Thumbnail>> {
    let path = thumbnail_path(&state, &id)?;
    let Ok(meta) = std::fs::metadata(&path) else {
        return Ok(None);
    };
    let written = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_millis() as u64);
    Ok(Some(Thumbnail { path, written }))
}

/// Everything kept about a world beside the shelf itself: what was said in it, how its
/// panes were arranged, its cameras - and, if it was the open one, that it was.
fn forget_world(
    store: &crate::store::Store,
    id: &str,
    remaining: &[crate::worlds::World],
) -> Reply<()> {
    store.delete_setting(&chat_key(id)).map_err(describe)?;
    store.delete_setting(&layout_key(id)).map_err(describe)?;
    store.delete_setting(&view_key(id)).map_err(describe)?;
    if store
        .setting(crate::worlds::ACTIVE)
        .ok()
        .flatten()
        .as_deref()
        == Some(id)
    {
        let next = remaining
            .first()
            .map(|world| world.id.clone())
            .unwrap_or_default();
        store
            .set_setting(crate::worlds::ACTIVE, &next)
            .map_err(describe)?;
    }
    Ok(())
}

/// Where a world's conversation with its agent is kept.
///
/// Beside the world rather than in it: the shelf reads every world to draw the row, and a
/// long conversation in each would be megabytes read for nothing. Keyed by the world's id,
/// so it is that world's and no other's - a conversation about one structure is not a
/// thing to find still open over a different one.
fn chat_key(world: &str) -> String {
    format!("chat:{world}")
}

/// The conversation held in a world, as the panel saved it. Empty if there has been none.
#[tauri::command]
pub fn world_chat(state: State<'_, AppState>, id: String) -> Reply<serde_json::Value> {
    let saved = state.store.setting(&chat_key(&id)).map_err(describe)?;
    Ok(saved
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({ "messages": [] })))
}

/// Saves a world's conversation. Refused for a world that does not exist, so a turn that
/// finishes after its world was deleted does not leave a conversation with no world.
#[tauri::command]
pub fn world_chat_save(
    state: State<'_, AppState>,
    id: String,
    chat: serde_json::Value,
) -> Reply<()> {
    if !all_worlds(&state).iter().any(|world| world.id == id) {
        return Err(format!("no world called {id}"));
    }
    let text = serde_json::to_string(&chat).map_err(describe)?;
    state
        .store
        .set_setting(&chat_key(&id), &text)
        .map_err(describe)
}

/// Opens a world: it becomes the one the viewer reads and writes.
#[tauri::command]
pub fn world_open(state: State<'_, AppState>, id: String) -> Reply<crate::worlds::World> {
    let mut worlds = all_worlds(&state);
    let world = worlds
        .iter_mut()
        .find(|world| world.id == id)
        .ok_or_else(|| format!("no world called {id}"))?;
    world.opened = crate::worlds::now_ms();
    let opened = world.clone();
    write_worlds(&state, &worlds)?;
    state
        .store
        .set_setting(crate::worlds::ACTIVE, &id)
        .map_err(describe)?;
    Ok(opened)
}

/// One world, graph and camera, for the shelf's preview.
#[tauri::command]
pub fn world_read(state: State<'_, AppState>, id: String) -> Reply<crate::worlds::World> {
    all_worlds(&state)
        .into_iter()
        .find(|world| world.id == id)
        .ok_or_else(|| format!("no world called {id}"))
}

/// Remembers where the camera was left.
#[tauri::command]
pub fn world_camera(
    state: State<'_, AppState>,
    id: String,
    camera: crate::worlds::Camera,
) -> Reply<()> {
    let mut worlds = all_worlds(&state);
    let world = worlds
        .iter_mut()
        .find(|world| world.id == id)
        .ok_or_else(|| format!("no world called {id}"))?;
    world.camera = Some(camera);
    write_worlds(&state, &worlds)
}

// --- agents ---------------------------------------------------------------------------
//
// The conversation and the tools live in the UI, where the graph and the viewport are;
// what lives here is the credential, which never crosses into the webview, and the
// adapters for the three wire formats. See `agents`.

/// The stored settings, with any connection the app has since learned to make added.
///
/// Without this, a kind added in a later version would never appear for anyone who had
/// already used the panel once - their stored list would simply not have it, and there is
/// no screen that says "here is a new sort of connection you could add".
fn agent_settings(state: &AppState) -> crate::agents::Settings {
    let mut settings: crate::agents::Settings = state
        .store
        .object_setting(crate::agents::SETTING)
        .unwrap_or_default();
    for seeded in crate::agents::default_connections() {
        if !settings
            .connections
            .iter()
            .any(|existing| existing.id == seeded.id)
        {
            settings.connections.push(seeded);
        }
    }
    // Claude and ChatGPT are offered through the user's subscription only. A list saved by
    // a version that also seeded key-based connections is brought in line, and written
    // down so the settings window shows what is actually there.
    if settings.keep_to_subscriptions() {
        if let Ok(text) = serde_json::to_string(&settings) {
            state.store.set_setting(crate::agents::SETTING, &text).ok();
        }
    }
    // An earlier version wrote the engine's port into the endpoint. That port belonged to
    // a process that is long gone, so the connection is turned back into what it meant:
    // "run this model with Geis's own engine", which is resolved when it is next used.
    for connection in &mut settings.connections {
        if connection.engine_model.is_some()
            || !matches!(connection.kind, crate::agents::Kind::Local)
        {
            continue;
        }
        if !connection.endpoint.contains("127.0.0.1") {
            continue;
        }
        connection.engine_model = crate::agents::weights::catalogue()
            .into_iter()
            .find(|model| model.file == connection.model)
            .map(|model| model.id);
        if connection.engine_model.is_none() {
            connection.endpoint = "http://localhost:11434/v1".to_string();
        }
    }
    settings
}

#[tauri::command]
pub fn agent_config(state: State<'_, AppState>) -> Reply<crate::agents::Settings> {
    Ok(agent_settings(&state))
}

/// Saves the connections and the policy in one go: they are one screen and one setting.
#[tauri::command]
pub fn agent_save_config(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    config: crate::agents::Settings,
) -> Reply<crate::agents::Settings> {
    let text = serde_json::to_string(&config).map_err(describe)?;
    state
        .store
        .set_setting(crate::agents::SETTING, &text)
        .map_err(describe)?;
    announce_agents(&app);
    Ok(config)
}

/// Tells every window that the agent's settings or its engine have changed.
///
/// The settings are their own window, so the panel that does the talking is not the one
/// that changed them: without this, pressing Use looked like nothing had happened, because
/// the conversation went on naming the model it had read at startup.
///
/// Sent after a beat, and not inline, because a broadcast from inside a command loses that
/// command's own reply: `engine_start` ran to the end - the server was up and answering -
/// and the promise in the window that had pressed Use never settled, so the button looked
/// dead while the thing it asked for had already happened.
fn announce_agents(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = app.emit("agents:changed", ());
    });
}

/// Which connections have a key in the keychain, so the panel can say what is ready.
#[tauri::command]
pub fn agent_ready(state: State<'_, AppState>) -> Reply<Vec<String>> {
    Ok(agent_settings(&state)
        .connections
        .into_iter()
        .filter(|connection| {
            connection.secret_ref.as_ref().map_or(true, |reference| {
                crate::legacy::read_secret(SECRET_SERVICE, reference).is_some()
            })
        })
        .map(|connection| connection.id)
        .collect())
}

fn connection(state: &AppState, id: &str) -> Result<crate::agents::Connection, String> {
    agent_settings(state)
        .connections
        .into_iter()
        .find(|connection| connection.id == id)
        .ok_or_else(|| format!("no connection called {id}"))
}

/// One turn of a conversation.
#[tauri::command]
pub async fn agent_complete(
    state: State<'_, AppState>,
    id: String,
    request: crate::agents::ChatRequest,
) -> Reply<crate::agents::ChatReply> {
    let mut connection = connection(&state, &id)?;
    // A connection Geis runs itself is addressed by whatever port its engine is on now,
    // and the engine is started if it is not up - after a restart, the first question
    // brings it back rather than failing at an address that nothing answers.
    if let Some(model) = connection.engine_model.clone() {
        let port = ensure_engine(&state, &model).await?;
        connection.endpoint = crate::agents::engine::endpoint(port);
    }
    crate::agents::complete(&connection, &request).await
}

/// One turn, reported as it arrives.
///
/// The deltas go down a channel rather than out as an event: they belong to one caller and
/// one turn, and a broadcast would reach windows that did not ask and - as the settings
/// window taught us - can lose the command's own reply.
#[tauri::command]
pub async fn agent_stream(
    state: State<'_, AppState>,
    id: String,
    request: crate::agents::ChatRequest,
    on_delta: tauri::ipc::Channel<crate::agents::stream::Delta>,
) -> Reply<crate::agents::ChatReply> {
    let mut connection = connection(&state, &id)?;
    if let Some(model) = connection.engine_model.clone() {
        let port = ensure_engine(&state, &model).await?;
        connection.endpoint = crate::agents::engine::endpoint(port);
    }
    crate::agents::complete_streaming(&connection, &request, |delta| {
        let _ = on_delta.send(delta);
    })
    .await
}

/// What models a connection offers - and the cheapest check that it is reachable.
#[tauri::command]
pub async fn agent_probe(state: State<'_, AppState>, id: String) -> Reply<Vec<String>> {
    let connection = connection(&state, &id)?;
    crate::agents::probe(&connection).await
}

/// Whether a subscription connection's tool is installed and signed in.
#[tauri::command]
pub async fn agent_account(
    state: State<'_, AppState>,
    id: String,
) -> Reply<crate::agents::cli::Account> {
    let connection = connection(&state, &id)?;
    Ok(crate::agents::cli_account(&connection).await)
}

/// Starts the tool's own sign-in in the browser and reports how it ended.
#[tauri::command]
pub async fn agent_sign_in(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Reply<crate::agents::cli::Account> {
    let connection = connection(&state, &id)?;
    let account = crate::agents::cli_login(&connection).await?;
    announce_agents(&app);
    Ok(account)
}

/// The index of the documents an agent can read: a name and a line each.
#[tauri::command]
pub fn agent_docs() -> Vec<crate::agents::docs::Doc> {
    crate::agents::docs::all()
        .into_iter()
        .map(|doc| crate::agents::docs::Doc {
            body: String::new(),
            ..doc
        })
        .collect()
}

#[tauri::command]
pub fn agent_doc(name: String) -> Reply<crate::agents::docs::Doc> {
    crate::agents::docs::by_name(&name).ok_or_else(|| format!("no document called {name}"))
}

// --- local models ----------------------------------------------------------------------
//
// The weights are not in the app bundle; they are fetched on a button. Everything about
// that is here: what is on offer, what is on this machine, and the download itself, which
// reports as it goes and can be stopped and picked up again.

/// The catalogue, with what this machine already has - and anything left behind.
#[tauri::command]
pub fn local_models(state: State<'_, AppState>) -> Vec<crate::agents::weights::ModelState> {
    crate::agents::weights::states(&state.paths.data_dir)
}

/// Stops a download in flight. The part-finished file stays, so it can be resumed.
#[tauri::command]
pub fn local_model_cancel(state: State<'_, AppState>, id: String) -> Reply<()> {
    if let Some(flag) = state.downloads.lock().unwrap().get(&id) {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// Removes the weights, and any half-finished download of them.
#[tauri::command]
pub fn local_model_delete(state: State<'_, AppState>, id: String) -> Reply<()> {
    let data = &state.paths.data_dir;
    // A file the catalogue no longer lists - a model from an older version of this list -
    // is still on the disk taking gigabytes, so it is removable by name.
    if let Some(name) = id.strip_prefix("file:") {
        let path = stray_weights(data, name)?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(describe)?;
        }
        return Ok(());
    }
    let model =
        crate::agents::weights::by_id(&id).ok_or_else(|| format!("no model called {id}"))?;
    for path in [
        crate::agents::weights::weights_path(data, &model),
        crate::agents::weights::part_path(data, &model),
    ] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(describe)?;
        }
    }
    Ok(())
}

/// The file a stray weights entry names: one `.gguf` directly in the weights directory.
///
/// The name comes from the webview. `..` passes a check on the parent directory - the
/// parent of `llm/..` is `llm` - so the name itself is held to being a plain file name.
fn stray_weights(data: &std::path::Path, name: &str) -> Reply<PathBuf> {
    let plain = matches!(
        std::path::Path::new(name)
            .components()
            .collect::<Vec<_>>()
            .as_slice(),
        [std::path::Component::Normal(_)]
    );
    if !plain || !name.ends_with(".gguf") || name.contains(['/', '\\']) {
        return Err("that is not a weights file".into());
    }
    Ok(crate::agents::weights::weights_dir(data).join(name))
}

/// Downloads a model's weights, reporting as it goes.
///
/// Written to a `.part` file and renamed at the end, so a file that exists is a whole one;
/// resumed with a Range request when a part is already there, because losing four gigabytes
/// to a closed laptop and having to start again is the thing that makes people give up.
#[tauri::command]
pub async fn local_model_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Reply<()> {
    use tauri::Emitter;

    let model =
        crate::agents::weights::by_id(&id).ok_or_else(|| format!("no model called {id}"))?;
    let data = state.paths.data_dir.clone();
    let directory = crate::agents::weights::weights_dir(&data);
    std::fs::create_dir_all(&directory).map_err(describe)?;
    let target = crate::agents::weights::weights_path(&data, &model);
    let part = crate::agents::weights::part_path(&data, &model);

    let report = |received: u64, total: u64, state: &str, error: Option<String>| {
        let _ = app.emit(
            "local-model:progress",
            crate::agents::weights::Progress {
                id: id.clone(),
                received,
                total,
                state: state.to_string(),
                error,
            },
        );
    };

    if target.exists() {
        let size = std::fs::metadata(&target)
            .map(|meta| meta.len())
            .unwrap_or(0);
        report(size, size, "done", None);
        return Ok(());
    }

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut downloads = state.downloads.lock().unwrap();
        if downloads.contains_key(&id) {
            return Err(format!("{} is already downloading", model.name));
        }
        downloads.insert(id.clone(), cancel.clone());
    }
    // Whatever happens below, the model must not stay marked as downloading.
    struct Guard<'a>(&'a AppState, String);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.downloads.lock().unwrap().remove(&self.1);
        }
    }
    let _guard = Guard(&state, id.clone());

    let already = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 60 * 6))
        .build()
        .map_err(describe)?;
    crate::agents::weights::fetch(
        crate::agents::weights::Fetch {
            client: &client,
            url: &model.url,
            part: &part,
            target: &target,
            already,
            fallback_size: model.size,
            cancel: &cancel,
        },
        report,
    )
    .await?;
    Ok(())
}

// --- the local engine ------------------------------------------------------------------
//
// Weights are data; something has to run them. See `agents::engine` for why that something
// is fetched rather than shipped.

/// The key the engine's own download is tracked under, beside the models'.
const ENGINE_DOWNLOAD: &str = "engine";

/// Starting the engine is one thing at a time. Two starts racing each see no engine, each
/// spawn one, and the loser's cleanup then kills the winner's process.
static ENGINE_START: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Is there an engine, and is it running?
#[tauri::command]
pub async fn engine_state(state: State<'_, AppState>) -> Reply<crate::agents::engine::EngineState> {
    let running = state
        .engine
        .lock()
        .await
        .as_ref()
        .map(|engine| (engine.port, engine.model.clone()));
    Ok(crate::agents::engine::state(&state.paths.data_dir, running))
}

/// Downloads the pinned llama.cpp build, checks its digest, and unpacks it.
#[tauri::command]
pub async fn engine_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Reply<crate::agents::engine::EngineState> {
    use tauri::Emitter;
    let asset = crate::agents::engine::asset().ok_or_else(|| {
        format!(
            "no engine build for {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let data = state.paths.data_dir.clone();
    let directory = crate::agents::engine::engine_dir(&data);
    std::fs::create_dir_all(&directory).map_err(describe)?;
    let archive = crate::agents::engine::archive_path(&data, asset);
    let part = archive.with_extension("part");

    let report = |received: u64, total: u64, phase: &str, error: Option<String>| {
        let _ = app.emit(
            "engine:progress",
            crate::agents::weights::Progress {
                id: "engine".to_string(),
                received,
                total,
                state: phase.to_string(),
                error,
            },
        );
    };

    // One install at a time. The settings window fires and forgets, so a second press of
    // Install would start a second download appending to the same part file - and the
    // checksum of two interleaved copies matches nothing.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut downloads = state.downloads.lock().unwrap();
        if downloads.contains_key(ENGINE_DOWNLOAD) {
            return Err("the engine is already being installed".into());
        }
        downloads.insert(ENGINE_DOWNLOAD.to_string(), cancel.clone());
    }
    struct Installing<'a>(&'a AppState);
    impl Drop for Installing<'_> {
        fn drop(&mut self) {
            self.0.downloads.lock().unwrap().remove(ENGINE_DOWNLOAD);
        }
    }
    let installing = Installing(&state);

    if crate::agents::engine::server_path(&data).is_none() {
        if !archive.exists() {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60 * 30))
                .build()
                .map_err(describe)?;
            let already = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
            crate::agents::weights::fetch(
                crate::agents::weights::Fetch {
                    client: &client,
                    url: &crate::agents::engine::url(asset),
                    part: &part,
                    target: &archive,
                    already,
                    fallback_size: asset.size,
                    cancel: &cancel,
                },
                report,
            )
            .await?;
        }
        // What was downloaded must be what was pinned. An engine that fails this is deleted
        // rather than kept, so a retry does not resume onto a bad file.
        let digest = crate::agents::engine::digest(&archive)?;
        if digest != asset.sha256 {
            std::fs::remove_file(&archive).ok();
            let message =
                format!("the engine download did not match its published checksum ({digest})");
            report(0, asset.size, "failed", Some(message.clone()));
            return Err(message);
        }
        report(asset.size, asset.size, "unpacking", None);
        crate::agents::engine::unpack(&archive, &directory).await?;
        std::fs::remove_file(&archive).ok();
    }

    let server = crate::agents::engine::server_path(&data)
        .ok_or_else(|| "the engine archive had no llama-server in it".to_string())?;
    crate::agents::engine::make_executable(&server)?;
    report(asset.size, asset.size, "done", None);
    drop(installing);
    engine_state(state).await
}

/// The port a model is being served on, starting the engine if it is not already up.
///
/// Called both by the button and by the next question after a restart, which is the point:
/// the engine is a process, and a process that has gone should come back on its own rather
/// than leaving an address that answers nothing.
async fn ensure_engine(state: &AppState, model: &str) -> Result<u16, String> {
    // Taken first, so a question asked while the model is still loading waits for it
    // rather than being handed a port nothing is listening on yet.
    let _starting = ENGINE_START.lock().await;
    if let Some(running) = state.engine.lock().await.as_mut() {
        // Still alive, not merely still recorded: a server killed for memory leaves its
        // entry behind, and the port in it answers nothing.
        let alive = matches!(running.child.try_wait(), Ok(None));
        if alive && running.model == model {
            return Ok(running.port);
        }
    }
    launch_engine(state, model).await
}

async fn start_engine(state: &AppState, model: &str) -> Result<u16, String> {
    let _starting = ENGINE_START.lock().await;
    launch_engine(state, model).await
}

/// Starts the server and waits for it. The caller holds `ENGINE_START`.
async fn launch_engine(state: &AppState, model: &str) -> Result<u16, String> {
    let data = state.paths.data_dir.clone();
    let entry =
        crate::agents::weights::by_id(model).ok_or_else(|| format!("no model called {model}"))?;
    let weights = crate::agents::weights::weights_path(&data, &entry);
    if !weights.exists() {
        return Err(format!("{} has not been downloaded yet", entry.name));
    }
    let server = crate::agents::engine::server_path(&data)
        .ok_or_else(|| "the engine has not been installed yet".to_string())?;

    // One at a time: a second server on the same weights would only compete for memory.
    if let Some(mut running) = state.engine.lock().await.take() {
        running.child.kill().await.ok();
    }

    let port = crate::agents::engine::free_port();
    let log = std::fs::File::create(crate::agents::engine::engine_dir(&data).join("engine.log"))
        .map_err(|error| error.to_string())?;
    let child = tokio::process::Command::new(&server)
        .args(crate::agents::engine::arguments(
            &weights,
            port,
            crate::agents::engine::choose_context(&weights),
        ))
        .stdout(std::process::Stdio::null())
        // Kept, because when a model fails to load this is the only place that says why.
        .stderr(std::process::Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not start the engine: {error}"))?;
    if let Some(id) = child.id() {
        std::fs::write(crate::agents::engine::pid_path(&data), id.to_string()).ok();
    }
    *state.engine.lock().await = Some(crate::RunningEngine {
        child,
        port,
        model: model.to_string(),
    });

    // Loading several gigabytes off a disk takes a while; failing early would be wrong.
    // ...but a server that has already exited is never going to answer, and sitting out
    // the three minutes for it - a bad weights file dies within a second - is worse than
    // useless: whatever else took the port in the meantime could answer in its place.
    let exited = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            if let Some(running) = state.engine.lock().await.as_mut() {
                if let Ok(Some(status)) = running.child.try_wait() {
                    return status;
                }
            }
        }
    };
    let outcome = tokio::select! {
        ready = crate::agents::engine::wait_until_ready(port, std::time::Duration::from_secs(180)) => ready,
        status = exited => Err(format!("the engine stopped while loading the model ({status})")),
    };
    if let Err(error) = outcome {
        if let Some(mut running) = state.engine.lock().await.take() {
            running.child.kill().await.ok();
        }
        std::fs::remove_file(crate::agents::engine::pid_path(&data)).ok();
        return Err(format!(
            "{error}. See engine.log in the app's data directory."
        ));
    }
    Ok(port)
}

/// Starts the engine on a downloaded model, and marks it as what the local connection runs.
#[tauri::command]
pub async fn engine_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model: String,
) -> Reply<crate::agents::engine::EngineState> {
    start_engine(&state, &model).await?;
    // The connection remembers the *model*, not the port: the port belongs to the process
    // and will be different next time.
    let mut config = agent_settings(&state);
    if let Some(local) = config
        .connections
        .iter_mut()
        .find(|connection| matches!(connection.kind, crate::agents::Kind::Local))
    {
        local.engine_model = Some(model.clone());
        if let Some(entry) = crate::agents::weights::by_id(&model) {
            local.model = entry.file;
        }
    }
    if let Ok(text) = serde_json::to_string(&config) {
        state.store.set_setting(crate::agents::SETTING, &text).ok();
    }
    announce_agents(&app);
    engine_state(state).await
}

#[tauri::command]
pub async fn engine_stop(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Reply<crate::agents::engine::EngineState> {
    if let Some(mut running) = state.engine.lock().await.take() {
        running.child.kill().await.ok();
    }
    std::fs::remove_file(crate::agents::engine::pid_path(&state.paths.data_dir)).ok();
    announce_agents(&app);
    engine_state(state).await
}

// --- views: selections as BioMol has them ---------------------------------------------
//
// A selection is a `View` - a level and indices - and everything that shows one (the
// outline, the card, the BioMol panel's tree, the sequence strip) derives from it. See
// `structure::index`.

/// Evaluates a selection spec against a structure.
#[tauri::command]
pub fn select_view(
    state: State<'_, AppState>,
    id: String,
    spec: crate::structure::index::Spec,
) -> Reply<crate::structure::index::View> {
    let structure = lookup(&state, &id)?;
    let table = crate::structure::index::IndexTable::from_structure(&structure);
    Ok(crate::structure::index::evaluate(&structure, &table, &spec))
}

/// The viewport's two masks for a view.
#[tauri::command]
pub fn view_mask(
    state: State<'_, AppState>,
    id: String,
    view: crate::structure::index::View,
) -> Reply<SelectionMask> {
    let structure = lookup(&state, &id)?;
    let table = crate::structure::index::IndexTable::from_structure(&structure);
    let (atoms, residues) = crate::structure::index::masks(&view.bounded(&table), &table);
    Ok(SelectionMask { atoms, residues })
}

/// What a view amounts to, for the card: how many of each level, and a name.
#[derive(Debug, Clone, Serialize)]
pub struct ViewSummary {
    pub level: crate::structure::index::Level,
    pub chains: u32,
    pub residues: u32,
    pub atoms: u32,
    /// `A_1 · LYS 7 - GLU 23 (17 res)`, or `3 chains · 2,341 atoms`.
    pub label: String,
    /// The chain ids the view touches, `A_1` form.
    pub chain_ids: Vec<String>,
    /// For an atom view, the residues the atoms belong to - so the tree can mark them. The
    /// outline mask deliberately leaves them out (a ribbon is not an atom), so they are
    /// reported here instead. Empty for other levels, which have them in the mask.
    pub residue_indices: Vec<u32>,
}

#[tauri::command]
pub fn describe_view(
    state: State<'_, AppState>,
    id: String,
    view: crate::structure::index::View,
) -> Reply<ViewSummary> {
    let structure = lookup(&state, &id)?;
    let table = crate::structure::index::IndexTable::from_structure(&structure);
    Ok(summarise_view(&structure, &table, &view.bounded(&table)))
}

fn summarise_view(
    structure: &crate::structure::Structure,
    table: &crate::structure::index::IndexTable,
    view: &crate::structure::index::View,
) -> ViewSummary {
    use crate::structure::index::Level;
    let chains = view.to_chains(table);
    let residues = view.to_residues(table);
    let atoms = view.to_atoms(table);
    let chain_ids: Vec<String> = chains
        .indices
        .iter()
        .filter_map(|chain| structure.chains.get(*chain as usize))
        .map(|chain| chain.id.clone())
        .collect();
    let residue_name = |index: u32| -> String {
        structure
            .residues
            .get(index as usize)
            .map(|residue| format!("{} {}", residue.name, residue.number))
            .unwrap_or_default()
    };
    let label = match view.level {
        Level::Chain => match chain_ids.as_slice() {
            [one] => format!(
                "{one} · {} res · {} atoms",
                residues.indices.len(),
                atoms.indices.len()
            ),
            many => format!("{} chains · {} atoms", many.len(), atoms.indices.len()),
        },
        Level::Residue => {
            let where_ = if chain_ids.len() == 1 {
                format!("{} · ", chain_ids[0])
            } else {
                String::new()
            };
            match residues.indices.as_slice() {
                [] => "nothing".to_string(),
                [one] => format!("{where_}{}", residue_name(*one)),
                many => format!(
                    "{where_}{} - {} ({} res)",
                    residue_name(*many.iter().min().unwrap()),
                    residue_name(*many.iter().max().unwrap()),
                    many.len()
                ),
            }
        }
        Level::Atom => match atoms.indices.as_slice() {
            [] => "nothing".to_string(),
            [one] => {
                let atom = *one as usize;
                let residue = table.atom_to_res.get(atom).copied().unwrap_or(0);
                format!(
                    "{} · {}{}",
                    residue_name(residue),
                    structure
                        .atom_names
                        .get(atom)
                        .map(|n| n.trim())
                        .unwrap_or(""),
                    structure
                        .elements
                        .get(atom)
                        .map(|e| format!(" ({})", crate::structure::chem::symbol(*e)))
                        .unwrap_or_default()
                )
            }
            many => format!("{} atoms in {} res", many.len(), residues.indices.len()),
        },
    };
    ViewSummary {
        level: view.level,
        chains: chains.indices.len() as u32,
        atoms: atoms.indices.len() as u32,
        label,
        chain_ids,
        residue_indices: if view.level == Level::Atom {
            residues.indices.clone()
        } else {
            Vec::new()
        },
        residues: residues.indices.len() as u32,
    }
}

/// One chain of the molecule map: its residues as parallel arrays, compact.
#[derive(Debug, Clone, Serialize)]
pub struct MapChain {
    pub id: String,
    pub auth_id: String,
    pub entity_type: String,
    /// Structure residue index of each residue, in chain order.
    pub residue_indices: Vec<u32>,
    /// One-letter codes, one per residue.
    pub codes: String,
    /// `H`, `E` or `-`, one per residue.
    pub secondary: String,
    /// Author numbers, one per residue.
    pub numbers: Vec<i32>,
    /// Mean B-factor per residue.
    pub b_factors: Vec<f32>,
    /// Three-letter names, one per residue - for the tooltip, and for ligands, whose
    /// one-letter code is `X`.
    pub names: Vec<String>,
    /// Where each residue stands when the structure is drawn as a graph of its
    /// components: x, y, z per residue. The alpha carbon of an amino acid, C4' of a
    /// nucleotide, and the centroid of anything else.
    pub centers: Vec<f32>,
    /// What each residue is, one character per residue: `a` amino acid, `n` nucleotide,
    /// `w` water, `i` a single atom that is not water (an ion), `l` any other ligand.
    pub kinds: String,
    /// Each residue's atoms, as the first one's index and how many there are - so that
    /// what is known per atom (the colour it is drawn in) can be asked per residue.
    pub first_atoms: Vec<u32>,
    pub atom_counts: Vec<u32>,
}

/// A link between two residues that is not the polymer backbone: a disulfide, a covalent
/// modification, a metal coordination. Drawn as a chord across the map.
#[derive(Debug, Clone, Serialize)]
pub struct MapLink {
    pub residues: [u32; 2],
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructureMap {
    pub id: String,
    pub entry_id: String,
    pub title: String,
    pub atom_count: u32,
    pub chains: Vec<MapChain>,
    pub links: Vec<MapLink>,
    /// The polymer bonds between consecutive residues, as pairs of residue indices. Kept
    /// apart from `links`, which are the bonds worth listing; these are the string the
    /// graph view threads a chain's components on.
    pub backbone: Vec<[u32; 2]>,
}

/// Everything the molecule map draws, in one call: a structure's chains with their
/// residues as parallel arrays, and the residue links that are not the backbone. Arrays
/// rather than objects per residue for the same reason `ChainSequence` uses strings: a
/// ribosome is a hundred thousand residues.
#[tauri::command]
pub async fn structure_map(state: State<'_, AppState>, id: String) -> Reply<StructureMap> {
    let structure = lookup(&state, &id)?;
    // Off the main thread, for the same reason a file is read off it.
    tauri::async_runtime::spawn_blocking(move || map_of(id, &structure))
        .await
        .map_err(describe)
}

fn map_of(id: String, structure: &crate::structure::Structure) -> StructureMap {
    use crate::structure::model::{BondKind, ResidueKind, SecondaryStructure};
    let chains = structure
        .chains
        .iter()
        .enumerate()
        .map(|(chain_index, chain)| {
            let rows: Vec<(usize, &crate::structure::model::Residue)> = structure
                .residues
                .iter()
                .enumerate()
                .filter(|(_, residue)| residue.chain_index as usize == chain_index)
                .collect();
            MapChain {
                id: chain.id.clone(),
                auth_id: chain.auth_id.clone(),
                entity_type: chain.entity_type.clone(),
                residue_indices: rows.iter().map(|(index, _)| *index as u32).collect(),
                codes: rows
                    .iter()
                    .map(|(_, r)| crate::structure::chem::residue_code(&r.name))
                    .collect(),
                secondary: rows
                    .iter()
                    .map(|(_, r)| match r.secondary {
                        SecondaryStructure::Helix => 'H',
                        SecondaryStructure::Sheet => 'E',
                        _ => '-',
                    })
                    .collect(),
                numbers: rows.iter().map(|(_, r)| r.number).collect(),
                b_factors: rows
                    .iter()
                    .map(|(_, r)| {
                        let first = r.first_atom as usize;
                        let end = (first + r.atom_count as usize).min(structure.b_factors.len());
                        if end > first {
                            structure.b_factors[first..end].iter().sum::<f32>()
                                / (end - first) as f32
                        } else {
                            0.0
                        }
                    })
                    .collect(),
                names: rows.iter().map(|(_, r)| r.name.clone()).collect(),
                centers: rows
                    .iter()
                    .flat_map(|(_, r)| component_center(structure, r))
                    .collect(),
                kinds: rows
                    .iter()
                    .map(|(_, r)| match r.kind {
                        ResidueKind::AminoAcid => 'a',
                        ResidueKind::Nucleotide => 'n',
                        ResidueKind::Water => 'w',
                        ResidueKind::Ligand if r.atom_count <= 1 => 'i',
                        ResidueKind::Ligand => 'l',
                    })
                    .collect(),
                first_atoms: rows.iter().map(|(_, r)| r.first_atom).collect(),
                atom_counts: rows.iter().map(|(_, r)| r.atom_count).collect(),
            }
        })
        .collect();
    let links = structure
        .residue_links
        .iter()
        .filter(|link| {
            !matches!(
                link.kind,
                BondKind::Link | BondKind::Template | BondKind::Inferred
            )
        })
        .map(|link| MapLink {
            residues: link.residues,
            kind: match link.kind {
                BondKind::Disulfide => "disulfide",
                BondKind::Covalent => "covalent",
                BondKind::Metal => "metal",
                _ => "other",
            }
            .to_string(),
        })
        .collect();
    let backbone = structure
        .residue_links
        .iter()
        .filter(|link| matches!(link.kind, BondKind::Link))
        .map(|link| link.residues)
        .collect();
    StructureMap {
        id,
        entry_id: structure.entry_id.clone(),
        title: structure.title.clone(),
        atom_count: structure.atom_count() as u32,
        chains,
        links,
        backbone,
    }
}

/// Where one component stands in the graph view: on the atom its polymer is traced
/// through where it has one, and at the middle of its atoms otherwise.
fn component_center(
    structure: &crate::structure::model::Structure,
    residue: &crate::structure::model::Residue,
) -> [f32; 3] {
    use crate::structure::model::ResidueKind;
    let first = residue.first_atom as usize;
    let end = (first + residue.atom_count as usize).min(structure.positions.len() / 3);
    let at = |atom: usize| {
        [
            structure.positions[atom * 3],
            structure.positions[atom * 3 + 1],
            structure.positions[atom * 3 + 2],
        ]
    };
    let named = |name: &str| {
        (first..end).find(|&atom| {
            structure
                .atom_names
                .get(atom)
                .is_some_and(|n| n.trim() == name)
        })
    };
    let anchor = match residue.kind {
        ResidueKind::AminoAcid => named("CA"),
        // Not the phosphate the ribbon runs through: C4' sits in the sugar, between the
        // backbone and the base, which is where a nucleotide looks like it is.
        ResidueKind::Nucleotide => named("C4'").or_else(|| named("C4*")),
        _ => None,
    };
    if let Some(atom) = anchor {
        return at(atom);
    }
    if end <= first {
        return [0.0; 3];
    }
    let mut sum = [0.0f32; 3];
    for atom in first..end {
        let p = at(atom);
        sum[0] += p[0];
        sum[1] += p[1];
        sum[2] += p[2];
    }
    let n = (end - first) as f32;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

/// One residue of a chain, as the BioMol panel's tree lists it.
#[derive(Debug, Clone, Serialize)]
pub struct ResidueRow {
    /// Index in the structure.
    pub index: u32,
    pub name: String,
    pub code: char,
    pub number: i32,
    pub insertion_code: Option<char>,
    /// `H`, `E` or `-`.
    pub secondary: char,
    /// `amino`, `nucleotide`, `water`, `ligand`.
    pub kind: String,
    pub atom_count: u32,
}

/// The residues of one chain, in the structure's order - not a range, since a chain's
/// residues are interleaved with other chains' in a deposition.
#[tauri::command]
pub fn chain_residues(
    state: State<'_, AppState>,
    id: String,
    chain: String,
) -> Reply<Vec<ResidueRow>> {
    use crate::structure::model::{ResidueKind, SecondaryStructure};
    let structure = lookup(&state, &id)?;
    let Some(chain_index) = structure
        .chains
        .iter()
        .position(|c| c.id.trim() == chain.trim())
    else {
        return Ok(Vec::new());
    };
    Ok(structure
        .residues
        .iter()
        .enumerate()
        .filter(|(_, residue)| residue.chain_index as usize == chain_index)
        .map(|(index, residue)| ResidueRow {
            index: index as u32,
            name: residue.name.clone(),
            code: crate::structure::chem::residue_code(&residue.name),
            number: residue.number,
            insertion_code: residue.insertion_code,
            secondary: match residue.secondary {
                SecondaryStructure::Helix => 'H',
                SecondaryStructure::Sheet => 'E',
                _ => '-',
            },
            kind: match residue.kind {
                ResidueKind::AminoAcid => "amino",
                ResidueKind::Nucleotide => "nucleotide",
                ResidueKind::Water => "water",
                ResidueKind::Ligand => "ligand",
            }
            .to_string(),
            atom_count: residue.atom_count,
        })
        .collect())
}

/// One atom of a residue, for the tree's deepest level.
#[derive(Debug, Clone, Serialize)]
pub struct AtomRow {
    pub index: u32,
    pub name: String,
    pub element: String,
    pub b_factor: f32,
}

/// A residue opened in the map: its atoms, and the bonds among them.
#[derive(Debug, Clone, Serialize)]
pub struct ResidueAtoms {
    pub atoms: Vec<AtomRow>,
    /// Pairs of structure atom indices, both within this residue.
    pub bonds: Vec<[u32; 2]>,
}

#[tauri::command]
pub fn residue_atoms(state: State<'_, AppState>, id: String, residue: u32) -> Reply<ResidueAtoms> {
    let structure = lookup(&state, &id)?;
    let Some(target) = structure.residues.get(residue as usize) else {
        return Ok(ResidueAtoms {
            atoms: Vec::new(),
            bonds: Vec::new(),
        });
    };
    let first = target.first_atom as usize;
    let end = (first + target.atom_count as usize).min(structure.atom_count());
    let atoms = (first..end)
        .map(|atom| AtomRow {
            index: atom as u32,
            name: structure
                .atom_names
                .get(atom)
                .map(|n| n.trim().to_string())
                .unwrap_or_default(),
            element: structure
                .elements
                .get(atom)
                .map(|e| crate::structure::chem::symbol(*e).to_string())
                .unwrap_or_default(),
            b_factor: structure.b_factors.get(atom).copied().unwrap_or(0.0),
        })
        .collect();
    let inside = |atom: u32| (atom as usize) >= first && (atom as usize) < end;
    let bonds = structure
        .bonds
        .iter()
        .filter(|bond| bond.is_covalent() && inside(bond.atoms[0]) && inside(bond.atoms[1]))
        .map(|bond| bond.atoms)
        .collect();
    Ok(ResidueAtoms { atoms, bonds })
}

/// Builds the mask the viewport highlights a selection with.
///
/// Computed here rather than in the UI because a chain's residues are not a range: a
/// deposition interleaves a chain's ligands and waters with other chains, so membership is
/// a test on every residue, and the core is where the residues are. Two masks, because the
/// spheres and sticks know which atom they are while a ribbon's vertices know only which
/// residue - see the viewer's `setSelection`.
///
/// An empty `chains` selects the whole structure, which is what selecting the molecule
/// rather than one of its chains means.
#[tauri::command]
pub fn selection_mask(
    state: State<'_, AppState>,
    id: String,
    chains: Vec<String>,
    residue: Option<u32>,
    atom: Option<u32>,
    residues: Option<Vec<u32>>,
) -> Reply<SelectionMask> {
    let structure = lookup(&state, &id)?;
    Ok(mask_for(
        &structure,
        &chains,
        residue,
        atom,
        residues.as_deref(),
    ))
}

/// The mask itself, apart from the command so it can be tested without an app.
///
/// The finer levels win: one atom, else one residue, else the named chains - and no chains
/// at all means the whole structure, which is what selecting the molecule means.
fn mask_for(
    structure: &crate::structure::Structure,
    chains: &[String],
    residue: Option<u32>,
    atom: Option<u32>,
    residue_set: Option<&[u32]>,
) -> SelectionMask {
    // A set of residues - a drag across the sequence strip.
    if let Some(set) = residue_set {
        let mut residues = vec![0u8; structure.residues.len()];
        for index in set {
            if let Some(slot) = residues.get_mut(*index as usize) {
                *slot = 1;
            }
        }
        let atoms = (0..structure.atom_count())
            .map(|index| {
                structure
                    .residue_of_atom
                    .get(index)
                    .and_then(|residue| residues.get(*residue as usize))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        return SelectionMask { atoms, residues };
    }
    if let Some(atom) = atom {
        let mut atoms = vec![0u8; structure.atom_count()];
        if let Some(slot) = atoms.get_mut(atom as usize) {
            *slot = 1;
        }
        // No ribbon segment lights up for one atom: the ribbon is a residue's, not an
        // atom's, and outlining a whole residue for an atom click would say the wrong
        // thing.
        return SelectionMask {
            atoms,
            residues: vec![0u8; structure.residues.len()],
        };
    }
    if let Some(residue) = residue {
        let mut residues = vec![0u8; structure.residues.len()];
        if let Some(slot) = residues.get_mut(residue as usize) {
            *slot = 1;
        }
        let atoms = (0..structure.atom_count())
            .map(|index| u8::from(structure.residue_of_atom.get(index).copied() == Some(residue)))
            .collect();
        return SelectionMask { atoms, residues };
    }

    let wanted: std::collections::HashSet<&str> = chains.iter().map(|chain| chain.trim()).collect();
    let chain_selected: Vec<bool> = structure
        .chains
        .iter()
        .map(|chain| wanted.is_empty() || wanted.contains(chain.id.trim()))
        .collect();
    let residues: Vec<u8> = structure
        .residues
        .iter()
        .map(|residue| {
            u8::from(
                chain_selected
                    .get(residue.chain_index as usize)
                    .copied()
                    .unwrap_or(false),
            )
        })
        .collect();
    let atoms: Vec<u8> = (0..structure.atom_count())
        .map(|atom| {
            structure
                .residue_of_atom
                .get(atom)
                .and_then(|residue| residues.get(*residue as usize))
                .copied()
                .unwrap_or(0)
        })
        .collect();
    SelectionMask { atoms, residues }
}

#[cfg(test)]
mod mask_tests {
    use super::*;
    use crate::structure::model::RawAtom;

    #[test]
    fn specs_select_by_chain_residue_and_atom_and_summarise() {
        use crate::structure::index::{
            evaluate, IndexTable, Level, NumberRange, Spec, View, Where,
        };
        let structure = structure();
        let table = IndexTable::from_structure(&structure);
        // Chain A by either name, at the residue level: its two residues.
        let spec = Spec {
            level: Level::Residue,
            r#where: Where {
                chains: vec!["A".into()],
                ..Default::default()
            },
        };
        assert_eq!(evaluate(&structure, &table, &spec).indices, vec![0, 1]);
        // CA atoms of residue 2 of chain A: one atom.
        let spec = Spec {
            level: Level::Atom,
            r#where: Where {
                chains: vec!["A_1".into()],
                residue_numbers: vec![NumberRange { from: 2, to: 2 }],
                atom_names: vec!["ca".into()],
                ..Default::default()
            },
        };
        assert_eq!(evaluate(&structure, &table, &spec).indices, vec![3]);
        // An empty spec at the chain level is every chain.
        let spec = Spec {
            level: Level::Chain,
            r#where: Where::default(),
        };
        assert_eq!(evaluate(&structure, &table, &spec).indices, vec![0, 1]);
        // Summaries name what was selected.
        let summary = summarise_view(&structure, &table, &View::new(Level::Residue, vec![0, 1]));
        assert_eq!(summary.label, "A_1 · ALA 1 - ALA 2 (2 res)");
        assert_eq!((summary.chains, summary.residues, summary.atoms), (1, 2, 4));
        let summary = summarise_view(&structure, &table, &View::new(Level::Atom, vec![3]));
        assert_eq!(summary.label, "ALA 2 · CA (C)");
    }

    fn structure() -> crate::structure::Structure {
        let atom = |name: &str, residue: i32, chain: &str| RawAtom {
            name: name.into(),
            element: 6,
            position: [0.0; 3],
            b_factor: 0.0,
            occupancy: 1.0,
            residue_name: "ALA".into(),
            residue_number: residue,
            insertion_code: None,
            chain_id: chain.into(),
            is_hetatm: false,
            label_asym_id: String::new(),
            entity_id: String::new(),
        };
        // Chain A: two residues of two atoms; chain B: one residue of two.
        crate::structure::Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("N", 1, "A"),
                atom("CA", 1, "A"),
                atom("N", 2, "A"),
                atom("CA", 2, "A"),
                atom("N", 1, "B"),
                atom("CA", 1, "B"),
            ],
        )
    }

    #[test]
    fn a_chain_a_residue_and_an_atom_are_three_different_masks() {
        let structure = structure();
        // The chain: both its residues, all four atoms.
        let chain = mask_for(&structure, &["A_1".into()], None, None, None);
        assert_eq!(chain.atoms, vec![1, 1, 1, 1, 0, 0]);
        assert_eq!(chain.residues, vec![1, 1, 0]);
        // One residue: its two atoms and its ribbon segment.
        let residue = mask_for(&structure, &[], Some(1), None, None);
        assert_eq!(residue.atoms, vec![0, 0, 1, 1, 0, 0]);
        assert_eq!(residue.residues, vec![0, 1, 0]);
        // One atom: that atom and no ribbon at all.
        let atom = mask_for(&structure, &[], None, Some(4), None);
        assert_eq!(atom.atoms, vec![0, 0, 0, 0, 1, 0]);
        assert_eq!(atom.residues, vec![0, 0, 0]);
        // Nothing named means everything.
        let all = mask_for(&structure, &[], None, None, None);
        assert!(all.atoms.iter().all(|flag| *flag == 1));
        // A set of residues, as a drag across the strip names them.
        let range = mask_for(&structure, &[], None, None, Some(&[0, 2]));
        assert_eq!(range.residues, vec![1, 0, 1]);
        assert_eq!(range.atoms, vec![1, 1, 0, 0, 1, 1]);
    }
}

/// The sequences of an open structure, for the sequence strip.
#[tauri::command]
pub fn structure_sequence(state: State<'_, AppState>, id: String) -> Reply<Vec<ChainSequence>> {
    use crate::structure::model::SecondaryStructure;
    let structure = lookup(&state, &id)?;

    let mut sequences = Vec::new();
    for (index, chain) in structure.chains.iter().enumerate() {
        let residues: Vec<_> = structure
            .residues
            .iter()
            .enumerate()
            .filter(|(_, residue)| residue.chain_index == index as u32)
            .collect();
        let residue_indices: Vec<u32> = residues.iter().map(|(at, _)| *at as u32).collect();
        let residues: Vec<_> = residues.into_iter().map(|(_, residue)| residue).collect();
        let codes: String = residues
            .iter()
            .map(|residue| crate::structure::chem::residue_code(&residue.name))
            .collect();
        // A chain of waters and ions has no sequence to show; a row of X pushes the real
        // chains off screen.
        if codes.chars().all(|code| code == 'X') {
            continue;
        }
        let secondary: String = residues
            .iter()
            .map(|residue| match residue.secondary {
                SecondaryStructure::Helix => 'H',
                SecondaryStructure::Sheet => 'E',
                _ => '-',
            })
            .collect();
        sequences.push(ChainSequence {
            chain: chain.id.clone(),
            codes,
            secondary,
            first_number: residues.first().map(|residue| residue.number).unwrap_or(0),
            residue_indices,
        });
    }
    Ok(sequences)
}

/// Where user-defined styles live between sessions.
const STYLES_SETTING: &str = "styles";

/// A style with a name, the way a Blender material is a named thing you can assign, keep
/// and come back to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedStyle {
    pub name: String,
    pub style: crate::geometry::Style,
    /// Shipped with the app, so it cannot be renamed or deleted.
    #[serde(default)]
    pub builtin: bool,
}

fn user_styles(state: &State<'_, AppState>) -> Vec<NamedStyle> {
    state.store.list_setting(STYLES_SETTING).unwrap_or_default()
}

fn write_user_styles(state: &State<'_, AppState>, styles: &[NamedStyle]) -> Reply<()> {
    let encoded = serde_json::to_string(styles).map_err(describe)?;
    state
        .store
        .set_setting(STYLES_SETTING, &encoded)
        .map_err(describe)
}

/// Saves a style under a name, replacing one of the same name.
///
/// A builtin cannot be overwritten: the presets are the floor to come back to, and losing
/// "Cartoon, secondary structure" to a stray click is not a thing that should be possible.
#[tauri::command]
pub fn style_save(
    state: State<'_, AppState>,
    name: String,
    style: crate::geometry::Style,
) -> Reply<Vec<NamedStyle>> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("a style needs a name".into());
    }
    if crate::geometry::presets()
        .iter()
        .any(|preset| preset.name == name)
    {
        return Err(format!("{name} is a built-in style; pick another name"));
    }

    let mut styles = user_styles(&state);
    match styles.iter_mut().find(|entry| entry.name == name) {
        Some(existing) => existing.style = style,
        None => styles.push(NamedStyle {
            name,
            style,
            builtin: false,
        }),
    }
    styles.sort_by(|a, b| a.name.cmp(&b.name));
    write_user_styles(&state, &styles)?;
    Ok(styles)
}

#[tauri::command]
pub fn style_delete(state: State<'_, AppState>, name: String) -> Reply<Vec<NamedStyle>> {
    let mut styles = user_styles(&state);
    styles.retain(|entry| entry.name != name);
    write_user_styles(&state, &styles)?;
    Ok(styles)
}

/// The named styles the app ships, and the named colours behind them.
///
/// Sent to the frontend rather than duplicated there: a preset is a `Style`, and a second
/// definition of what "Cartoon, orange" means would drift from this one.
#[derive(Debug, Serialize)]
pub struct StyleLibrary {
    /// Builtins first, then the user's, each already named.
    pub styles: Vec<NamedStyle>,
    pub colours: Vec<crate::geometry::NamedColour>,
}

#[tauri::command]
pub fn style_library(state: State<'_, AppState>) -> StyleLibrary {
    let mut styles: Vec<NamedStyle> = crate::geometry::presets()
        .into_iter()
        .map(|preset| NamedStyle {
            name: preset.name,
            style: preset.style,
            builtin: true,
        })
        .collect();
    styles.extend(user_styles(&state));
    StyleLibrary {
        styles,
        colours: crate::geometry::named_colours(),
    }
}

/// Everything the Add menu offers.
#[tauri::command]
pub fn graph_catalog() -> Vec<NodeTemplate> {
    use crate::geometry::{LightKind, Material, SceneOptions};
    use crate::graph::NodeKind;

    let template = |group: &str, kind: NodeKind| NodeTemplate {
        type_name: kind.type_name().to_string(),
        title: kind.title(),
        group: group.to_string(),
        kind,
    };

    vec![
        template(
            "Source",
            NodeKind::Molecule {
                version: Default::default(),
                pdb_id: String::new(),
                path: None,
                assembly: None,
                options: SceneOptions::default(),
                chains: Vec::new(),
                expanded: false,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            },
        ),
        template(
            "Source",
            NodeKind::SequenceInput {
                fasta: String::new(),
            },
        ),
        template(
            "Compute",
            NodeKind::Predict {
                model_id: "boltz2".into(),
                provider_id: "local".into(),
                inputs: std::collections::BTreeMap::new(),
                job_id: None,
            },
        ),
        template(
            "Style",
            NodeKind::Style {
                style: crate::geometry::Style::default(),
            },
        ),
        template(
            "Style",
            NodeKind::Colouring {
                scheme: crate::geometry::ColorScheme::Secondary,
                categorical: crate::geometry::Categorical::default(),
                continuous: false,
            },
        ),
        template(
            "Style",
            NodeKind::SolidColour {
                // PyMOL's orange, which is the one people name first.
                color: crate::geometry::color::to_linear([255, 128, 0]),
            },
        ),
        template(
            "Style",
            NodeKind::Material {
                material: Material::default(),
            },
        ),
        template(
            "Scene",
            NodeKind::Light {
                light: crate::geometry::Light {
                    id: "new".into(),
                    name: "Light".into(),
                    kind: LightKind::Sun,
                    color: [1.0, 1.0, 1.0],
                    energy: 1.5,
                    direction: [-0.4, 0.3, 0.866],
                    distance: 4.0,
                    softness: 10.0,
                    follow_view: true,
                    enabled: true,
                },
            },
        ),
        template(
            "Scene",
            NodeKind::World {
                background: [1.0, 1.0, 1.0],
                ambient: 0.55,
                ambient_occlusion: true,
                outline: false,
            },
        ),
        template("Output", NodeKind::Viewport),
        template("Output", NodeKind::Image { path: None }),
        template(
            "Output",
            NodeKind::Render {
                engine: crate::graph::RenderEngine::Cycles,
                samples: 64,
                width: 1600,
                height: 1200,
                transparent: false,
                output: None,
            },
        ),
    ]
}

/// The result of evaluating a graph and resolving whatever it asked for.
#[derive(Debug, Serialize)]
pub struct GraphResult {
    pub evaluation: crate::graph::Evaluation,
    pub nodes: Vec<NodeDescription>,
    /// The structures the plan resolved to, one per molecule, in the plan's order.
    pub structures: Vec<StructureInfo>,
    /// A job this evaluation started, so the editor can record it on the node. Without
    /// this the next evaluation would submit the same prediction again.
    pub submitted: Option<SubmittedJob>,
    /// The chains each opened structure turned out to have, for its Molecule group to
    /// record. A group cannot offer a socket per chain until it knows what they are, and
    /// only opening the structure can say.
    pub chains: Vec<DiscoveredChains>,
}

#[derive(Debug, Serialize)]
pub struct DiscoveredChains {
    pub node_id: String,
    pub chains: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SubmittedJob {
    pub node_id: String,
    pub job_id: String,
}

/// Evaluates a graph and does what it says.
///
/// Evaluation itself is pure - see `graph::eval` - and this is the half that touches the
/// world: submitting a prediction, fetching from the PDB, opening a file. Keeping them
/// apart is what lets the interesting cases be tested without a network or a GPU.
#[tauri::command]
pub async fn graph_evaluate(
    state: State<'_, AppState>,
    graph: crate::graph::Graph,
    // Whether a prediction that has not run may be started. The worlds shelf evaluates
    // every world only to draw it, and must not launch a GPU job by looking.
    submit: Option<bool>,
) -> Reply<GraphResult> {
    use crate::graph::NodeState;

    let statuses = job_statuses(&state, &graph);
    let mut evaluation = crate::graph::evaluate(&graph, &statuses);

    // A prediction node that has never run is the one case where evaluating means starting
    // work. Submitted here rather than in the pure pass so the same graph can be evaluated
    // repeatedly without launching a job each time.
    let mut submitted = None;
    for node in &graph.nodes {
        let is_pending = matches!(
            evaluation.node_states.get(&node.id),
            Some(NodeState::Incomplete { reason }) if reason == "not run yet"
        );
        if !is_pending || submit == Some(false) {
            continue;
        }
        let Some(mut request) = graph.job_request(node) else {
            continue;
        };
        // The job belongs to the world whose graph asked for it.
        request.world_id = Some(active_world_id(&state, &all_worlds(&state)));
        let manifest = {
            let registry = state.registry.read().unwrap();
            registry.get(&request.model_id).cloned()
        };
        let Some(manifest) = manifest else {
            // Said on the node. Skipped in silence it sits at "not run yet" and nothing
            // anywhere says why pressing run does nothing.
            evaluation.node_states.insert(
                node.id.clone(),
                NodeState::Failed {
                    message: format!("no model called {} is installed", request.model_id),
                },
            );
            continue;
        };
        match crate::compute::job::plan(&manifest, &request, &state.paths.jobs_dir) {
            Ok(spec) => match state.queue.submit(spec) {
                Ok(record) => {
                    evaluation.node_states.insert(
                        node.id.clone(),
                        NodeState::Running {
                            job_id: record.id.clone(),
                        },
                    );
                    submitted = Some(SubmittedJob {
                        node_id: node.id.clone(),
                        job_id: record.id,
                    });
                }
                Err(error) => {
                    evaluation.node_states.insert(
                        node.id.clone(),
                        NodeState::Failed {
                            message: error.to_string(),
                        },
                    );
                }
            },
            Err(error) => {
                evaluation.node_states.insert(
                    node.id.clone(),
                    NodeState::Failed {
                        message: error.to_string(),
                    },
                );
            }
        }
        break; // one submission per evaluation keeps the reported state unambiguous
    }

    // Resolve every part's structure. One failure takes the whole plan down: half a
    // scene with no explanation looks like the graph rather than like an error.
    // All at once: a world of several molecules was read one file after another. The
    // same reference twice - one entry shown as two molecules - is read once.
    let mut structures = Vec::new();
    if let Some(plan) = &evaluation.plan {
        let mut distinct: Vec<&crate::graph::StructureRef> = Vec::new();
        for part in &plan.parts {
            if !distinct.contains(&&part.structure) {
                distinct.push(&part.structure);
            }
        }
        let opened: Vec<Result<StructureInfo, String>> = all(distinct
            .iter()
            .map(|reference| {
                Box::pin(open_reference(&state, reference, &statuses)) as Pending<'_, _>
            })
            .collect())
        .await;
        let mut failed = None;
        for part in &plan.parts {
            let at = distinct
                .iter()
                .position(|reference| **reference == part.structure)
                .unwrap_or(0);
            match &opened[at] {
                Ok(info) => structures.push(info.clone()),
                Err(error) => {
                    failed = Some(error.clone());
                    break;
                }
            }
        }
        if let Some(error) = failed {
            evaluation.problems.push(error);
            evaluation.plan = None;
            structures.clear();
        }
    }

    // Chains go back to the group that asked for each structure, if they have changed.
    // Paired by position: the plan's parts and the opened structures are the same list.
    let mut chains = Vec::new();
    for (molecule, info) in molecules_for_output(&graph).into_iter().zip(&structures) {
        let found: Vec<String> = info.chains.iter().map(|chain| chain.id.clone()).collect();
        if let crate::graph::NodeKind::Molecule { chains: known, .. } = &molecule.kind {
            if *known != found {
                chains.push(DiscoveredChains {
                    node_id: molecule.id.clone(),
                    chains: found,
                });
            }
        }
    }

    Ok(GraphResult {
        nodes: describe_nodes(&graph),
        evaluation,
        structures,
        submitted,
        chains,
    })
}

/// The molecules the viewport draws, one per plan part, in the plan's order.
///
/// Reached through the chain nodes, since those are what the output takes, and de-duped:
/// four chains of one structure are one molecule and one part.
fn molecules_for_output(graph: &crate::graph::Graph) -> Vec<&crate::graph::Node> {
    let Ok(output) = graph.output_node() else {
        return Vec::new();
    };
    let mut seen: Vec<&crate::graph::Node> = Vec::new();
    for chain in graph.sources(&output.id, "Geometry") {
        if let Some(molecule) = graph.molecule_of(&chain.id) {
            if !seen.iter().any(|node| node.id == molecule.id) {
                seen.push(molecule);
            }
        }
    }
    seen
}

/// Job states for every prediction node in the graph that has already run.
///
/// Gathered here rather than inside the evaluator because reading the store is exactly
/// what the evaluator must not do; it takes these as an argument and stays pure.
fn job_statuses(
    state: &State<'_, AppState>,
    graph: &crate::graph::Graph,
) -> std::collections::BTreeMap<String, crate::graph::JobStatus> {
    use crate::graph::{JobStatus, NodeKind};

    let mut statuses = std::collections::BTreeMap::new();
    for node in &graph.nodes {
        if let NodeKind::Predict {
            job_id: Some(job_id),
            ..
        } = &node.kind
        {
            // A job this database has never heard of - a world carried over from another
            // machine, a jobs table cleared - is not one that is still running. Leaving
            // it out reads as "running" to the evaluator, and the node then waits for
            // ever on a prediction nobody is making.
            let Ok(record) = state.store.job(job_id) else {
                statuses.insert(
                    job_id.clone(),
                    JobStatus {
                        job_id: job_id.clone(),
                        state: crate::compute::job::JobState::Failed,
                        structure: None,
                        error: Some(
                            "this prediction's job is not on this machine - clear the node's job to run it again"
                                .into(),
                        ),
                    },
                );
                continue;
            };
            {
                let structure = state
                    .store
                    .artifacts(job_id)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|artifact| artifact.kind == "structure")
                    .map(|artifact| artifact.path);
                statuses.insert(
                    job_id.clone(),
                    JobStatus {
                        job_id: job_id.clone(),
                        state: record.state,
                        structure,
                        error: record.error,
                    },
                );
            }
        }
    }
    statuses
}

/// A future to wait on beside others.
type Pending<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Waits for every one of `futures`, letting them run at once, and gives their results in
/// the order they were given.
async fn all<T>(futures: Vec<Pending<'_, T>>) -> Vec<T> {
    let mut futures: Vec<Option<Pending<'_, T>>> = futures.into_iter().map(Some).collect();
    let mut results: Vec<Option<T>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut waiting = false;
        for (slot, future) in futures.iter_mut().enumerate() {
            if let Some(running) = future {
                match running.as_mut().poll(context) {
                    std::task::Poll::Ready(value) => {
                        results[slot] = Some(value);
                        *future = None;
                    }
                    std::task::Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    results.into_iter().flatten().collect()
}

/// Turns a resolved structure reference into an open structure, whatever it points at.
async fn open_reference(
    state: &State<'_, AppState>,
    reference: &crate::graph::StructureRef,
    statuses: &std::collections::BTreeMap<String, crate::graph::JobStatus>,
) -> Result<StructureInfo, String> {
    use crate::graph::StructureRef;
    match reference {
        StructureRef::File {
            path,
            assembly,
            version,
        } => {
            open_structure(
                state.clone(),
                path.clone(),
                Some(*assembly),
                Some(version.into()),
            )
            .await
        }
        StructureRef::Pdb {
            id,
            assembly,
            version,
        } => {
            fetch_structure(
                state.clone(),
                id.clone(),
                Some(*assembly),
                Some(version.into()),
            )
            .await
        }
        StructureRef::Job { job_id } => match statuses
            .get(job_id)
            .and_then(|status| status.structure.clone())
        {
            Some(path) => open_structure(state.clone(), path, None, None).await,
            None => Err("the prediction has no structure to show".to_string()),
        },
    }
}

/// The camera a render is taken from. Comes from the viewport, so a render is the view on
/// screen at higher quality rather than a second guess at the same framing.
#[derive(Debug, Clone, Deserialize)]
pub struct CameraView {
    pub eye: [f32; 3],
    pub center: [f32; 3],
    pub up: [f32; 3],
    pub radius: f32,
    #[serde(default = "default_camera_fov")]
    pub fov_degrees: f32,
    /// The viewport's width/height, so the render can frame what the viewport framed.
    #[serde(default)]
    pub view_aspect: f32,
}

fn default_camera_fov() -> f32 {
    40.0
}

/// Renders one Render node in the graph.
///
/// The node's own inputs decide the scene and its own fields decide the quality, so two
/// Render nodes in one graph produce two different figures. The plan comes from the same
/// `graph::resolve` the viewport used - resolving it a second way here is how a render
/// starts to disagree with the preview it was set up in.
#[tauri::command]
pub async fn graph_render(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    graph: crate::graph::Graph,
    node_id: String,
    camera: CameraView,
    output: Option<PathBuf>,
) -> Reply<crate::render::RenderOutcome> {
    use crate::graph::NodeKind;

    let node = graph
        .node(&node_id)
        .ok_or_else(|| format!("no node called {node_id}"))?;
    let NodeKind::Render {
        engine,
        samples,
        width,
        height,
        transparent,
        output: last,
        ..
    } = &node.kind
    else {
        return Err(format!("{node_id} is not a Render node"));
    };
    // Rendering again replaces this node's last render, as Blender's Render Result is
    // replaced: a new numbered file each time filled the image list with untitled-1,
    // untitled-2 and so on for what was one picture made over. Only a file the app made
    // itself is reused; one the user named is theirs, and a new render does not write
    // over it unasked.
    let renders = state.paths.data_dir.join("renders");
    let output = output.or_else(|| {
        last.as_ref()
            .filter(|path| path.parent() == Some(renders.as_path()) && path.is_file())
            .cloned()
    });

    let statuses = job_statuses(&state, &graph);
    let resolved = crate::graph::resolve(&graph, node, &statuses);
    let plan = resolved.plan.ok_or_else(|| {
        resolved
            .problems
            .first()
            .cloned()
            .unwrap_or_else(|| "the Render node has nothing to render".to_string())
    })?;
    let mut parts = Vec::new();
    for part in &plan.parts {
        let structure = open_reference(&state, &part.structure, &statuses).await?;
        parts.push(ScenePart {
            structure_id: structure.id,
            options: part.options,
            assignments: part.assignments.clone(),
            features: part.features.clone(),
            place: None,
            size: None,
        });
    }

    let request = RenderRequestBody {
        parts,
        style: plan.style,
        output,
        settings: crate::render::RenderSettings {
            eye: camera.eye,
            center: camera.center,
            up: camera.up,
            fov_degrees: camera.fov_degrees,
            view_aspect: camera.view_aspect,
            radius: camera.radius,
            width: *width,
            height: *height,
            samples: *samples,
            engine: engine.as_str().to_string(),
            transparent: *transparent,
            draft: false,
        },
    };
    render_scene(app, state, request).await
}

// --- offline rendering ------------------------------------------------------

/// Whether Blender is available, and which one.
#[tauri::command]
pub async fn blender_info() -> Option<crate::render::BlenderInfo> {
    crate::render::info().await
}

/// A free `untitled.png` in a directory, numbering only when it has to.
///
/// Renders used to be named with a UUID, which is unimpeachable as a key and useless as a
/// filename: the Save-as dialog offered `4f3a....png`, and the file someone ended up with
/// said nothing about anything. `untitled.png` is what an unnamed document is called
/// everywhere else, and the counter appears only from the second one.
fn next_untitled(directory: &std::path::Path) -> std::path::PathBuf {
    let candidate = directory.join("untitled.png");
    if !candidate.exists() {
        return candidate;
    }
    for index in 1..10_000 {
        let candidate = directory.join(format!("untitled-{index}.png"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Ten thousand untitled renders in one directory is not a case that deserves a better
    // answer than a unique one.
    directory.join(format!("untitled-{}.png", util::new_id()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenderRequestBody {
    pub parts: Vec<ScenePart>,
    #[serde(default)]
    pub style: crate::geometry::SceneStyle,
    pub settings: crate::render::RenderSettings,
    /// Where to write the image. Defaults to a file in the app's render directory.
    #[serde(default)]
    pub output: Option<PathBuf>,
}

/// Renders the current view with Blender.
///
/// Same geometry as the viewport, tessellated and path-traced. The point is not a
/// prettier picture of the same thing - it is ambient occlusion and soft shadows, which
/// are what make contacts and cavities readable and which a real-time renderer cannot do.
#[tauri::command]
pub async fn render_scene(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: RenderRequestBody,
) -> Reply<crate::render::RenderOutcome> {
    // The same merge the viewport does, so a render of a two-molecule scene is that
    // scene rather than one of its halves.
    let mut scene = crate::geometry::Scene::empty();
    for part in &request.parts {
        let structure = lookup(&state, &part.structure_id)?;
        scene.append(crate::geometry::build_scene_with(
            &structure,
            part.options,
            &part.assignments,
            &part.features,
            &request.style,
            // A render is the opposite of a preview: full detail, always.
            false,
        ));
    }
    scene.settle();

    let renders = state.paths.data_dir.join("renders");
    // An unnamed render claims its name now, with an empty file, rather than when the
    // image arrives. Two renders started together would otherwise both be `untitled.png`
    // - the name is only taken once Blender finishes - and one would replace the other.
    let reserved = request.output.is_none();
    let output = match request.output {
        Some(output) => output,
        None => {
            std::fs::create_dir_all(&renders).map_err(describe)?;
            loop {
                let candidate = next_untitled(&renders);
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                {
                    Ok(_) => break candidate,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(describe(error)),
                }
            }
        }
    };
    let workdir = renders.join("work");

    // How far Blender has got, said as it goes - a render is minutes, and a line saying
    // only "Rendering..." for all of them is no better than nothing. Keyed by where the
    // picture will land, which is what the window asked for.
    let key = output.display().to_string();
    let outcome = crate::render::render_reporting(
        &scene,
        &request.style,
        &request.settings,
        &workdir,
        &output,
        |progress| {
            use tauri::Emitter;
            let _ = app.emit(
                "render:progress",
                serde_json::json!({ "output": key, "fraction": progress.fraction, "stage": progress.stage }),
            );
        },
    )
    .await;
    if outcome.is_err() && reserved {
        // The placeholder is not a picture; a failed render leaves nothing behind.
        let _ = std::fs::remove_file(&output);
    }
    outcome.map_err(describe)
}

/// Writes the scene on screen as a 3D model for slides: a `.glb`, which PowerPoint, Word
/// and Keynote take with its colours.
///
/// Spheres and sticks are turned into triangles the way they are for a Blender render. A
/// large structure is exported at the preview's detail: a slide's 3D viewer is not built
/// for tens of millions of triangles, and at a slide's size the finer ribbon is not seen.
#[tauri::command]
pub async fn export_glb(
    state: State<'_, AppState>,
    request: SceneRequest,
    path: PathBuf,
) -> Reply<GlbExport> {
    let mut opened = Vec::with_capacity(request.parts.len());
    for part in &request.parts {
        opened.push(lookup(&state, &part.structure_id)?);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let residues: usize = opened
            .iter()
            .map(|structure| structure.residues.len())
            .sum();
        let coarse = residues > 30_000;
        let mut scene = crate::geometry::Scene::empty();
        for (part, structure) in request.parts.iter().zip(&opened) {
            scene.append(crate::geometry::representation::build_scene_among(
                structure,
                part.options,
                &part.assignments,
                &part.features,
                &request.style,
                coarse,
                residues,
            ));
        }
        scene.settle();
        // Spheres and sticks at the lighter division: haemoglobin in sticks is 24 MB at
        // it and 70 at the finer one, and a slide does not show the difference.
        let mesh = crate::geometry::tessellate(&scene, crate::geometry::Quality::Draft);
        if mesh.indices.is_empty() {
            return Err("there is nothing drawn to export".to_string());
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("molecule")
            .to_string();
        let bytes = crate::geometry::gltf::encode_glb(&mesh, &name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(describe)?;
        }
        std::fs::write(&path, &bytes).map_err(describe)?;
        Ok(GlbExport {
            path,
            triangles: mesh.triangle_count() as u64,
            bytes: bytes.len() as u64,
        })
    })
    .await
    .map_err(describe)?
}

/// What an export wrote: where, and how much.
#[derive(Debug, Serialize)]
pub struct GlbExport {
    pub path: PathBuf,
    pub triangles: u64,
    pub bytes: u64,
}

/// Stops every render under way: the status bar's Cancel.
#[tauri::command]
pub fn render_cancel() {
    crate::render::CANCEL.notify_waiters();
}

/// A render requested by the environment rather than by a click, for scripted figures and
/// for verifying the bridge without a person at the keyboard.
#[tauri::command]
pub fn blender_render_target() -> Option<PathBuf> {
    std::env::var("GEIS_BLENDER_RENDER").ok().map(PathBuf::from)
}

// --- images -----------------------------------------------------------------

/// Copies a rendered image to wherever the user asked for it.
///
/// A copy rather than a re-render: the file already exists in the app's render directory,
/// and rendering it a second time to a new path would take minutes and could differ.
#[tauri::command]
pub fn export_image(from: PathBuf, to: PathBuf) -> Reply<PathBuf> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    std::fs::copy(&from, &to).map_err(describe)?;
    Ok(to)
}

/// Puts an image on the system clipboard.
///
/// Done here rather than with the webview's clipboard API, which needs the write to happen
/// inside the user gesture that started it. Reading the bytes first loses that gesture, and
/// WebKit then resolves the promise anyway - so the browser path reported success and
/// copied nothing. The system clipboard has no such rule.
#[tauri::command]
pub fn copy_image(path: PathBuf) -> Reply<()> {
    if !path.is_file() {
        return Err(format!("{} is not there", path.display()));
    }

    #[cfg(target_os = "macos")]
    {
        // AppleScript is the only route to the pasteboard without linking AppKit. The
        // path goes into a quoted literal, so a quote or a backslash in it has to be
        // escaped or the script silently becomes a different script.
        let quoted = path
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script =
            format!("set the clipboard to (read (POSIX file \"{quoted}\") as «class PNGf»)");
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(describe)?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("copying an image to the clipboard is not implemented on this platform".into())
    }
}

/// Fetches a picture from the web into the app's cache and says where it is.
///
/// For the agent's `show_image`: a figure it has found is shown in a pane, and a pane
/// shows files. Only https, only something that is a picture by its own account, and not
/// past a size no figure reaches - this is a download made on a model's say-so.
#[tauri::command]
pub async fn fetch_image(state: State<'_, AppState>, url: String) -> Reply<PathBuf> {
    use sha2::{Digest, Sha256};
    let parsed = image_url(&url)?;
    let client = reqwest::Client::builder()
        // Publishers turn away a client that does not say what it is.
        .user_agent("Mozilla/5.0 (Macintosh) geis-structure-viewer")
        .timeout(std::time::Duration::from_secs(40))
        // "Only https" has to hold for where the address leads, not only for the address.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                attempt.error("the address redirected away from https")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(describe)?;
    let mut response = client
        .get(parsed.clone())
        .header("Accept", "image/*")
        .send()
        .await
        .map_err(|error| {
            format!(
                "could not reach {}: {error}",
                parsed.host_str().unwrap_or("the site")
            )
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "{} answered {} - try the address of the image itself, or another copy of the figure",
            parsed.host_str().unwrap_or("the site"),
            response.status()
        ));
    }
    let kind = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let extension = image_extension(kind)?;
    // Refused before the body is read when the server says how big it is, and counted as
    // it arrives when it does not. Reading it all first and measuring after meant an
    // address that streamed without end was held in memory until the timeout.
    if response
        .content_length()
        .is_some_and(|length| length > IMAGE_LIMIT)
    {
        return Err("that image is larger than 40 MB".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("download failed: {error}"))?
    {
        if body.len() as u64 + chunk.len() as u64 > IMAGE_LIMIT {
            return Err("that image is larger than 40 MB".into());
        }
        body.extend_from_slice(&chunk);
    }
    let name = Sha256::digest(parsed.as_str().as_bytes())
        .iter()
        .take(10)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = state.paths.data_dir.join("cache/images");
    std::fs::create_dir_all(&directory).map_err(describe)?;
    let path = directory.join(format!("{name}.{extension}"));
    std::fs::write(&path, &body).map_err(describe)?;
    Ok(path)
}

const IMAGE_LIMIT: u64 = 40 * 1024 * 1024;

/// The address of a picture to fetch, if it is one this app will fetch.
fn image_url(url: &str) -> Reply<reqwest::Url> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|error| format!("that is not a URL: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("only https addresses are fetched".into());
    }
    Ok(parsed)
}

/// The file extension for a picture's stated type, or why it is not a picture.
fn image_extension(content_type: &str) -> Reply<&'static str> {
    let kind = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Ok(match kind.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        other => {
            return Err(format!(
                "that address is {} and not a picture - give the address of the image file itself, not of the page it is on",
                if other.is_empty() { "of no stated type" } else { other }
            ))
        }
    })
}

/// Saves a rendered frame. The UI hands over a data URL from the canvas.
#[tauri::command]
pub fn save_image(path: PathBuf, data_url: String) -> Reply<PathBuf> {
    use base64::Engine;
    let encoded = data_url
        .split_once(";base64,")
        .map(|(_, data)| data)
        .ok_or("that is not a base64 data URL")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("could not decode the image: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    std::fs::write(&path, bytes).map_err(describe)?;
    Ok(path)
}

/// A view the environment asked for, rather than one a user clicked together.
///
/// One command for both outputs. They were separate, and the representation and colour
/// were attached to the screenshot - so asking only for a Blender render silently got the
/// default colouring. Anything that describes *what to draw* belongs in one place.
#[derive(Debug, Serialize)]
pub struct ScriptedView {
    /// Representation name, or None for the viewer's default.
    pub representation: Option<String>,
    pub color_scheme: Option<String>,
    /// Where to write a screenshot of the viewport, if asked for.
    pub screenshot: Option<PathBuf>,
    /// Where to write a Blender render, if asked for.
    pub blender_render: Option<PathBuf>,
}

/// Reads `GEIS_REPRESENTATION`, `GEIS_COLOR`, `GEIS_SCREENSHOT` and
/// `GEIS_BLENDER_RENDER`.
///
/// This is how figures get produced in a batch, and how the renderers are verified with
/// nobody at the keyboard.
#[tauri::command]
pub fn scripted_view() -> Option<ScriptedView> {
    let representation = std::env::var("GEIS_REPRESENTATION").ok();
    let color_scheme = std::env::var("GEIS_COLOR").ok();
    let screenshot = std::env::var("GEIS_SCREENSHOT").ok().map(PathBuf::from);
    let blender_render = std::env::var("GEIS_BLENDER_RENDER").ok().map(PathBuf::from);

    if representation.is_none()
        && color_scheme.is_none()
        && screenshot.is_none()
        && blender_render.is_none()
    {
        return None;
    }
    Some(ScriptedView {
        representation,
        color_scheme,
        screenshot,
        blender_render,
    })
}

// --- diagnostics ------------------------------------------------------------

/// Lets the webview report to stderr, where `tauri dev` shows it.
///
/// A frontend that throws during boot leaves a blank window and no explanation anywhere -
/// the webview's console is not visible from the terminal. This is the channel that makes
/// "nothing is showing" a diagnosable state instead of a dead end.
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    eprintln!("[ui:{level}] {message}");
}

// --- files ------------------------------------------------------------------

/// Show a job's directory in the OS file manager. The escape hatch for anything the UI
/// does not surface yet - and the reason nothing here needs to be a black box.
#[tauri::command]
pub fn reveal_path(app: tauri::AppHandle, path: PathBuf) -> Reply<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(&path).map_err(describe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_is_packed_as_flat_numbers() {
        let residues = PresetResult::Residues(vec![3, 70000]).encode();
        assert_eq!(residues.len(), 16);
        assert_eq!(&residues[0..8], &[0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(
            u32::from_le_bytes(residues[12..16].try_into().unwrap()),
            70000
        );

        let contact = crate::structure::contacts::Contact {
            residues: [4, 9],
            names: ["a".into(), "b".into()],
            distance: 2.9,
            from: [1.0, 2.0, 3.0],
            to: [4.0, 5.0, 6.0],
        };
        let contacts = PresetResult::Contacts(vec![contact]).encode();
        assert_eq!(contacts.len(), 8 + 36);
        assert_eq!(u32::from_le_bytes(contacts[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(contacts[12..16].try_into().unwrap()), 9);
        assert_eq!(
            f32::from_le_bytes(contacts[16..20].try_into().unwrap()),
            2.9
        );
        assert_eq!(
            f32::from_le_bytes(contacts[40..44].try_into().unwrap()),
            6.0
        );
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("geis-commands-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_world_id_cannot_put_a_conversation_outside_the_bin() {
        let root = scratch();
        let bin = root.join("tmp/chats");
        for id in ["../../escaped", "/etc/x", "a/b", "", "..", "a b", "x\0y"] {
            assert!(
                put_in_bin(&bin, id, b"{}", 1024).is_err(),
                "{id:?} should be refused"
            );
            assert!(take_from_bin(&bin, id).is_err());
        }
        assert!(!root.join("escaped.json").exists());
        assert!(std::fs::read_dir(&root)
            .unwrap()
            .all(|entry| entry.unwrap().file_name() == "tmp"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_bin_gives_back_the_newest_and_empties_the_oldest() {
        let root = scratch();
        let bin = root.join("chats");
        let world = "6f9c2f1e-0000-4000-8000-000000000001";
        put_in_bin(&bin, world, br#"{"n":1}"#, 1 << 20).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        put_in_bin(&bin, world, br#"{"n":2}"#, 1 << 20).unwrap();
        put_in_bin(&bin, "other", br#"{"n":9}"#, 1 << 20).unwrap();
        assert_eq!(take_from_bin(&bin, world).unwrap().unwrap()["n"], 2);
        assert_eq!(take_from_bin(&bin, world).unwrap().unwrap()["n"], 1);
        assert!(take_from_bin(&bin, world).unwrap().is_none());
        assert!(take_from_bin(&root.join("never-made"), world)
            .unwrap()
            .is_none());

        // Past the limit the oldest go, but never the one just put in, however large.
        let big = vec![b' '; 4096];
        put_in_bin(&bin, "a", &big, 5000).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        put_in_bin(&bin, "b", &big, 5000).unwrap();
        assert!(
            take_from_bin(&bin, "a").unwrap().is_none(),
            "the oldest was emptied"
        );
        assert!(std::fs::read_dir(&bin).unwrap().count() >= 1);
        put_in_bin(&bin, "c", &vec![b' '; 10_000], 5000).unwrap();
        let left: Vec<String> = std::fs::read_dir(&bin)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left.len(), 1);
        assert!(left[0].ends_with("-c.json"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn deleting_a_world_takes_everything_kept_beside_it() {
        let store = crate::store::Store::open_in_memory().unwrap();
        let keep = crate::worlds::World::new("Keep".into(), crate::graph::default_graph());
        let gone = "gone-world";
        for key in [
            chat_key(gone),
            layout_key(gone),
            view_key(gone),
            chat_key(&keep.id),
            layout_key(&keep.id),
        ] {
            store.set_setting(&key, "{}").unwrap();
        }
        // A world whose id is a prefix of the deleted one's is a different world.
        store.set_setting(&chat_key("gone-world-2"), "{}").unwrap();
        store.set_setting(crate::worlds::ACTIVE, gone).unwrap();

        forget_world(&store, gone, std::slice::from_ref(&keep)).unwrap();
        for key in [chat_key(gone), layout_key(gone), view_key(gone)] {
            assert!(
                store.setting(&key).unwrap().is_none(),
                "{key} was left behind"
            );
        }
        assert!(store.setting(&chat_key(&keep.id)).unwrap().is_some());
        assert!(store.setting(&layout_key(&keep.id)).unwrap().is_some());
        assert!(store.setting(&chat_key("gone-world-2")).unwrap().is_some());
        assert_eq!(
            store.setting(crate::worlds::ACTIVE).unwrap().as_deref(),
            Some(keep.id.as_str())
        );

        // Deleting one that is not open leaves the open one alone.
        forget_world(&store, "another", &[]).unwrap();
        assert_eq!(
            store.setting(crate::worlds::ACTIVE).unwrap().as_deref(),
            Some(keep.id.as_str())
        );
    }

    #[test]
    fn a_log_is_never_cut_through_a_character() {
        let root = scratch();
        let path = root.join("job.log");
        assert_eq!(
            read_log_slice(&path, None).unwrap().next_offset,
            0,
            "no log yet is not an error"
        );

        // The job is half-way through writing "결과": the last read must stop before it.
        let mut bytes = b"step 1\n".to_vec();
        bytes.extend_from_slice(&"결과".as_bytes()[..4]);
        std::fs::write(&path, &bytes).unwrap();
        let first = read_log_slice(&path, Some(0)).unwrap();
        assert_eq!(first.text, "step 1\n결");
        assert_eq!(first.next_offset, 10);
        // The rest arrives, and the next read starts on the character it stopped before.
        bytes.extend_from_slice(&"결과".as_bytes()[4..]);
        std::fs::write(&path, &bytes).unwrap();
        let second = read_log_slice(&path, Some(first.next_offset)).unwrap();
        assert_eq!(second.text, "과");
        assert!(!format!("{}{}", first.text, second.text).contains('\u{fffd}'));

        // A log replaced by a shorter one starts again from the top.
        std::fs::write(&path, b"new").unwrap();
        assert_eq!(read_log_slice(&path, Some(999)).unwrap().text, "new");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_tail_of_a_long_log_starts_on_a_whole_character() {
        let root = scratch();
        let path = root.join("job.log");
        // Three-byte characters, so the tail's first byte is unlikely to begin one.
        let text = "가".repeat((LOG_TAIL as usize / 3) + 10);
        std::fs::write(&path, text.as_bytes()).unwrap();
        let tail = read_log_slice(&path, None).unwrap();
        assert!(!tail.text.contains('\u{fffd}'));
        assert!(tail.text.chars().all(|c| c == '가'));
        assert_eq!(tail.next_offset, text.len() as u64);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_stray_weights_name_is_one_file_in_the_weights_directory() {
        let data = std::path::Path::new("/data");
        let ok = stray_weights(data, "old-model.gguf").unwrap();
        assert_eq!(
            ok,
            crate::agents::weights::weights_dir(data).join("old-model.gguf")
        );
        for bad in [
            "..",
            "../geis.sqlite",
            "../../x.gguf",
            "/etc/x.gguf",
            "sub/x.gguf",
            "",
            ".",
            "x.txt",
            "..\\x.gguf",
        ] {
            assert!(
                stray_weights(data, bad).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn only_https_pictures_are_fetched() {
        assert!(image_url("https://example.org/fig1.png").is_ok());
        assert!(image_url("  https://example.org/fig1.png  ").is_ok());
        for bad in [
            "http://example.org/a.png",
            "file:///etc/passwd",
            "ftp://x/a.png",
            "not a url",
            "",
            "javascript:alert(1)",
        ] {
            assert!(image_url(bad).is_err(), "{bad:?} should be refused");
        }
        assert_eq!(image_extension("image/png").unwrap(), "png");
        assert_eq!(
            image_extension("IMAGE/JPEG; charset=binary").unwrap(),
            "jpg"
        );
        assert_eq!(image_extension(" image/svg+xml ").unwrap(), "svg");
        assert!(image_extension("text/html; charset=utf-8").is_err());
        assert!(image_extension("").unwrap_err().contains("no stated type"));
        assert!(image_extension("image/png/../../x").is_err());
    }

    #[test]
    fn the_untitled_name_counts_up_only_when_it_has_to() {
        let root = scratch();
        assert_eq!(next_untitled(&root), root.join("untitled.png"));
        std::fs::write(root.join("untitled.png"), b"").unwrap();
        std::fs::write(root.join("untitled-1.png"), b"").unwrap();
        assert_eq!(next_untitled(&root), root.join("untitled-2.png"));
        std::fs::remove_dir_all(&root).ok();
    }
}

/// What the file viewer shows for a file, worked out from what kind of file it is.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilePreview {
    /// Shown as it is: the window reads it through the asset protocol.
    Image {
        path: PathBuf,
    },
    Pdf {
        path: PathBuf,
    },
    /// A document turned into a page: Word, RTF, OpenDocument text.
    Html {
        html: String,
    },
    /// Plain text, up to the first megabyte.
    Text {
        text: String,
        cut: bool,
    },
    /// A picture of the first page or slide, where the file is not something the window
    /// can draw: a presentation, a spreadsheet.
    Picture {
        path: PathBuf,
    },
    /// Nothing the viewer can show; it can still be opened in its own application.
    Other,
}

/// The file viewer's picture of a file.
///
/// Documents go through what the system already has rather than a library of our own:
/// `textutil` reads Word, RTF and OpenDocument into a page, and Quick Look - Finder's
/// space-bar preview - draws a presentation's or a spreadsheet's first page.
#[tauri::command]
pub async fn file_preview(state: State<'_, AppState>, path: PathBuf) -> Reply<FilePreview> {
    let previews = state.paths.data_dir.join("previews");
    tauri::async_runtime::spawn_blocking(move || preview_file(&path, &previews))
        .await
        .map_err(describe)?
}

/// A PDF's bytes, for the file viewer to draw. Up to 300 MB: past that a file is not one
/// to read in a pane.
#[tauri::command]
pub async fn file_bytes(path: PathBuf) -> Reply<tauri::ipc::Response> {
    tauri::async_runtime::spawn_blocking(move || {
        let size = std::fs::metadata(&path).map_err(describe)?.len();
        if size > 300 << 20 {
            return Err(format!("{} is too large to show here", path.display()));
        }
        std::fs::read(&path)
            .map(tauri::ipc::Response::new)
            .map_err(describe)
    })
    .await
    .map_err(describe)?
}

fn preview_file(path: &Path, previews: &Path) -> Reply<FilePreview> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    Ok(match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => FilePreview::Image {
            path: path.to_path_buf(),
        },
        "pdf" => FilePreview::Pdf {
            path: path.to_path_buf(),
        },
        "docx" | "doc" | "rtf" | "rtfd" | "odt" | "wordml" | "webarchive" => {
            let output = std::process::Command::new("/usr/bin/textutil")
                .args(["-convert", "html", "-stdout", "-noload", "-nostore"])
                .arg(path)
                .output()
                .map_err(describe)?;
            if !output.status.success() {
                return Err(format!(
                    "could not read {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            FilePreview::Html {
                html: String::from_utf8_lossy(&output.stdout).into_owned(),
            }
        }
        "pptx" | "ppt" | "key" | "xlsx" | "xls" | "numbers" | "pages" | "heic" | "tif" | "tiff" => {
            quick_look(path, previews)?
        }
        _ => read_text(path)?.unwrap_or(FilePreview::Other),
    })
}

/// Text, if the file reads as text: the first megabyte, and whether there was more.
fn read_text(path: &Path) -> Reply<Option<FilePreview>> {
    use std::io::Read;
    const LIMIT: u64 = 1 << 20;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(describe)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(describe)?;
    let cut = bytes.len() as u64 > LIMIT;
    bytes.truncate(LIMIT as usize);
    // A NUL is never in text, and nearly always early in anything else.
    if bytes.iter().take(8192).any(|&byte| byte == 0) {
        return Ok(None);
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        // Cut in the middle of a character, or not UTF-8 at all.
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            if !cut || valid + 4 < LIMIT as usize {
                return Ok(None);
            }
            let mut bytes = error.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).map_err(describe)?
        }
    };
    Ok(Some(FilePreview::Text { text, cut }))
}

/// Quick Look's picture of a file, made once per version of it.
fn quick_look(path: &Path, previews: &Path) -> Reply<FilePreview> {
    std::fs::create_dir_all(previews).map_err(describe)?;
    // Named for the file and when it was written, so an edited file gets a new picture
    // and an unchanged one is not drawn again.
    let written = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_secs());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(path, written), &mut hasher);
    let stamp = std::hash::Hasher::finish(&hasher);
    let folder = previews.join(format!("{stamp:016x}"));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let picture = folder.join(format!("{name}.png"));
    if !picture.is_file() {
        std::fs::create_dir_all(&folder).map_err(describe)?;
        let status = std::process::Command::new("/usr/bin/qlmanage")
            .args(["-t", "-s", "2000", "-o"])
            .arg(&folder)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(describe)?;
        if !status.success() || !picture.is_file() {
            return Ok(FilePreview::Other);
        }
    }
    Ok(FilePreview::Picture { path: picture })
}

#[cfg(test)]
mod file_preview_tests {
    use super::*;

    /// A folder of its own, removed when the test is done with it.
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("geis-preview-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Folder(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str, bytes: &[u8]) -> (Folder, PathBuf) {
        let dir = Folder::new();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn text_is_shown_as_text_and_anything_binary_is_not() {
        let (dir, path) = scratch("notes.cif", b"data_1UBQ\n_entry.id 1UBQ\n");
        match preview_file(&path, dir.path()).unwrap() {
            FilePreview::Text { text, cut } => {
                assert!(text.starts_with("data_1UBQ"));
                assert!(!cut);
            }
            other => panic!("{other:?}"),
        }
        let (dir, path) = scratch("blob.bin", &[0, 1, 2, 3, 0, 255]);
        assert!(matches!(
            preview_file(&path, dir.path()).unwrap(),
            FilePreview::Other
        ));
    }

    #[test]
    fn pictures_and_pdfs_are_handed_to_the_window_as_they_are() {
        let (dir, path) = scratch("figure.PNG", b"not really a png");
        assert!(matches!(
            preview_file(&path, dir.path()).unwrap(),
            FilePreview::Image { .. }
        ));
        let (dir, path) = scratch("paper.pdf", b"%PDF-1.4");
        assert!(matches!(
            preview_file(&path, dir.path()).unwrap(),
            FilePreview::Pdf { .. }
        ));
    }

    #[test]
    fn a_word_processor_document_becomes_a_page() {
        if !Path::new("/usr/bin/textutil").exists() {
            return;
        }
        let (dir, path) = scratch("letter.rtf", br"{\rtf1\ansi Dear reviewer}");
        match preview_file(&path, dir.path()).unwrap() {
            FilePreview::Html { html } => assert!(html.contains("Dear reviewer"), "{html}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_folder_is_not_a_file() {
        let dir = Folder::new();
        assert!(preview_file(dir.path(), dir.path()).is_err());
    }
}
