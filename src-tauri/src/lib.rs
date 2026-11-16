//! An end-to-end workbench for structure prediction: worlds to look at results in, and a
//! compute layer that hides where the work actually happened.
//!
//! Module map:
//!   models/   declarative model manifests - the plug-in point for any engine
//!   compute/  probe, plan, schedule, and execute jobs on local / ssh / slurm / http
//!   store     one SQLite database for worlds, providers, jobs, artifacts
//!   commands  the Tauri surface the UI talks to

pub mod agents;
pub mod commands;
pub mod compute;
pub mod geometry;
pub mod gpu;
pub mod graph;
pub mod legacy;
mod menu;
pub mod models;
pub mod render;
pub mod store;
pub mod structure;
pub mod util;
mod window_frame;
pub mod worlds;

use compute::exec::LogChunk;
use compute::provider::{ExecOptions, LocalConfig, ProviderConfig, ProviderRecord, ProviderStatus};
use compute::queue::{Events, Queue};
use models::Registry;
use store::{JobRecord, Store};

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager};

/// Keychain service name. Provider credentials are stored under this, never in the
/// database - a shared or backed-up SQLite file must not carry API keys.
pub const SECRET_SERVICE: &str = "com.sanggeunpark.geis.compute";

/// Everywhere the app keeps files.
#[derive(Debug, Clone)]
pub struct Paths {
    /// SQLite database, settings, caches.
    pub data_dir: PathBuf,
    /// One directory per job: staged inputs, the log, and results.
    pub jobs_dir: PathBuf,
    /// User-authored manifests. Shadows bundled ones by id, so a shipped model can be
    /// tweaked without editing the app.
    pub user_models_dir: PathBuf,
    /// Manifests shipped with the app.
    pub bundled_models_dir: Option<PathBuf>,
}

impl Paths {
    pub fn resolve(app: &AppHandle) -> Self {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("geis"));
        // Before anything is opened: a user's data from when the app was called mini.
        match legacy::carry_over(&data_dir) {
            Ok(legacy::Carried::Moved { rewritten }) => eprintln!(
                "carried the data from {} over to {} ({rewritten} stored paths repointed)",
                legacy::OLD_IDENTIFIER,
                data_dir.display()
            ),
            Ok(legacy::Carried::Nothing) => {}
            Err(error) => eprintln!("could not carry the old data over: {error}"),
        }
        let bundled_models_dir = resolve_bundled_models(app);
        Self {
            jobs_dir: data_dir.join("jobs"),
            user_models_dir: data_dir.join("models"),
            bundled_models_dir,
            data_dir,
        }
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("geis.sqlite")
    }

    pub fn manifest_dirs(&self) -> Vec<PathBuf> {
        // Bundled first so the user directory wins on an id collision.
        let mut dirs = Vec::new();
        if let Some(bundled) = &self.bundled_models_dir {
            dirs.push(bundled.clone());
        }
        dirs.push(self.user_models_dir.clone());
        dirs
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.jobs_dir)?;
        std::fs::create_dir_all(&self.user_models_dir)?;
        Ok(())
    }
}

/// Bundled manifests live in the app resources when packaged, and in `models/` at the
/// repo root during development. `GEIS_MODELS_DIR` overrides both, which is how one tests
/// a manifest without reinstalling anything.
fn resolve_bundled_models(app: &AppHandle) -> Option<PathBuf> {
    if let Some(path) = legacy::env_path("MODELS_DIR") {
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("models");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let dev_candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("models"))?;
    dev_candidate.is_dir().then_some(dev_candidate)
}

/// A parsed structure that is being kept.
pub struct OpenStructure {
    /// The id handed to the UI.
    pub id: String,
    pub structure: std::sync::Arc<structure::Structure>,
    /// The file it was read from and when that file was last written, so asking for the
    /// same file again finds this rather than parsing it a second time.
    pub source: Option<(PathBuf, Option<std::time::SystemTime>)>,
}

pub struct AppState {
    /// Structures that are open, keyed by the id handed to the UI.
    ///
    /// Parsing 6VXX takes most of a second, so re-reading the file every time someone
    /// changes representation is not an option. Bounded and oldest-first so a session that
    /// opens dozens of models does not hold them all.
    pub structures: std::sync::Mutex<Vec<OpenStructure>>,
    /// A structure path given on the command line, waiting for the UI to be ready for it.
    ///
    /// Pulled by the frontend rather than pushed as an event: `setup` runs before the
    /// webview has any listeners attached, so an event emitted here would be sent to
    /// nobody.
    pub pending_open: std::sync::Mutex<Option<PathBuf>>,
    pub paths: Paths,
    pub store: Arc<Store>,
    pub registry: RwLock<Registry>,
    pub queue: Arc<Queue>,
    /// Weight downloads in flight, by model id, each with the flag that stops it.
    pub downloads:
        std::sync::Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    /// The local inference server, when one is running.
    pub engine: Arc<tokio::sync::Mutex<Option<RunningEngine>>>,
}

/// A llama.cpp server this app started, and what it was started with.
pub struct RunningEngine {
    pub child: tokio::process::Child,
    pub port: u16,
    /// The model id, so the panel can say what is answering.
    pub model: String,
}

impl AppState {
    pub fn reload_models(&self) -> usize {
        let registry = Registry::load_dirs(&self.paths.manifest_dirs());
        let count = registry.len();
        *self.registry.write().unwrap() = registry;
        count
    }
}

/// Bridges the queue's progress reports onto Tauri events.
struct TauriEvents {
    app: AppHandle,
}

impl Events for TauriEvents {
    fn job_changed(&self, record: &JobRecord) {
        let _ = self.app.emit("job:changed", record);
    }

    fn job_log(&self, chunk: LogChunk) {
        let _ = self.app.emit("job:log", chunk);
    }

    fn provider_changed(&self, provider_id: &str) {
        let _ = self.app.emit("provider:changed", provider_id);
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .menu(menu::build)
        .on_menu_event(|app, event| menu::handle(app, event.id().as_ref()))
        // The local engine is a child process, and a child process outlives its parent
        // unless something kills it. Nobody wants a llama-server holding six gigabytes
        // after the window it belonged to has gone.
        .on_window_event(|window, event| {
            // Only the main window closing ends the session. A second window - the agent's
            // settings - closing is not the end of anything, and this handler used to run
            // for it too, asking for a kind of state that is not managed: the panic took
            // the whole app down with the settings window.
            if window.label() != "main" {
                return;
            }
            if matches!(
                event,
                tauri::WindowEvent::Moved(_)
                    | tauri::WindowEvent::Resized(_)
                    | tauri::WindowEvent::CloseRequested { .. }
            ) {
                remember_frame(
                    window,
                    matches!(event, tauri::WindowEvent::CloseRequested { .. }),
                );
            }
            if !matches!(event, tauri::WindowEvent::Destroyed) {
                return;
            }
            let engine = window.state::<AppState>().engine.clone();
            tauri::async_runtime::block_on(async move {
                if let Some(mut running) = engine.lock().await.take() {
                    running.child.kill().await.ok();
                }
            });
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let paths = Paths::resolve(&handle);
            paths.ensure()?;
            // A server from a run that was force-quit is still holding its model.
            agents::engine::kill_stale(&paths.data_dir);

            let store = Arc::new(Store::open(&paths.database())?);
            // A job cannot outlive the process that was streaming it, so anything left
            // "running" is from a crash or a force-quit.
            match store.reap_interrupted_jobs() {
                Ok(count) if count > 0 => {
                    eprintln!("marked {count} interrupted job(s) as failed");
                }
                _ => {}
            }
            seed_local_provider(&store)?;

            let registry = Registry::load_dirs(&paths.manifest_dirs());
            for problem in registry.problems() {
                eprintln!("manifest {}: {}", problem.path.display(), problem.message);
            }
            eprintln!(
                "{} model(s) loaded from {:?}",
                registry.len(),
                paths.manifest_dirs()
            );

            // Tauri's async runtime is the one every command handler and event emitter
            // already runs on; the queue is handed its handle so it spawns there rather
            // than looking for an ambient runtime it will not find in this hook.
            let queue = Queue::new(
                Arc::clone(&store),
                Arc::new(TauriEvents {
                    app: handle.clone(),
                }),
                tauri::async_runtime::handle().inner().clone(),
            );

            warn_if_dev_server_is_missing(&handle);
            restore_frame(&handle, &store);
            if std::env::var_os("GEIS_PRINT_MENU").is_some() {
                if let Some(menu) = app.menu() {
                    for line in menu::describe(&menu) {
                        eprintln!("[menu] {line}");
                    }
                }
            }

            app.manage(AppState {
                structures: std::sync::Mutex::new(Vec::new()),
                pending_open: std::sync::Mutex::new(
                    OPENED_EARLY
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                        .or_else(structure_path_from_args),
                ),
                paths,
                store,
                registry: RwLock::new(registry),
                queue,
                downloads: std::sync::Mutex::new(std::collections::HashMap::new()),
                engine: Arc::new(tokio::sync::Mutex::new(None)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::probe_provider,
            commands::list_models,
            commands::reload_models,
            commands::submit_job,
            commands::cancel_job,
            commands::extension_states,
            commands::extension_maintain,
            commands::list_jobs,
            commands::job_detail,
            commands::read_job_log,
            commands::set_secret,
            commands::has_secret,
            commands::delete_secret,
            commands::reveal_path,
            commands::pending_open,
            commands::blender_info,
            commands::render_scene,
            commands::render_cancel,
            commands::export_glb,
            commands::save_image,
            commands::fetch_image,
            commands::export_image,
            commands::copy_image,
            commands::scripted_view,
            commands::open_structure,
            commands::fetch_structure,
            commands::build_scene,
            commands::scene_legend,
            commands::color_swatches,
            commands::default_style,
            commands::worlds_list,
            commands::world_active,
            commands::world_create,
            commands::world_rename,
            commands::world_delete,
            commands::world_open,
            commands::world_read,
            commands::world_camera,
            commands::world_chat,
            commands::world_chat_save,
            commands::preferences,
            commands::preferences_save,
            commands::find_contacts,
            commands::find_core,
            commands::structure_preset,
            commands::chat_bin_put,
            commands::chat_bin_take,
            commands::agent_skills,
            commands::world_view,
            commands::world_view_save,
            commands::world_thumbnail,
            commands::file_preview,
            commands::file_bytes,
            commands::world_thumbnail_save,
            commands::world_layout,
            commands::world_layout_save,
            commands::graph_load,
            commands::graph_save,
            commands::graph_reset,
            commands::graph_catalog,
            commands::structure_sequence,
            commands::selection_mask,
            commands::pick_target,
            commands::select_view,
            commands::view_mask,
            commands::describe_view,
            commands::chain_residues,
            commands::residue_atoms,
            commands::structure_map,
            commands::agent_config,
            commands::agent_save_config,
            commands::agent_ready,
            commands::agent_complete,
            commands::agent_stream,
            commands::agent_probe,
            commands::agent_account,
            commands::agent_sign_in,
            commands::agent_docs,
            commands::agent_doc,
            commands::local_models,
            commands::local_model_download,
            commands::local_model_cancel,
            commands::local_model_delete,
            commands::engine_state,
            commands::engine_install,
            commands::engine_start,
            commands::engine_stop,
            commands::layout_load,
            commands::layout_save,
            commands::style_library,
            commands::style_save,
            commands::style_delete,
            commands::graph_evaluate,
            commands::graph_render,
            commands::close_structure,
            commands::ui_log,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start")
        .run(|app, event| {
            // A structure opened from Finder - double-clicked, or dropped on the Dock icon -
            // does not arrive as an argument on macOS but as this event, and it can arrive
            // before the window has loaded or long after. So it is both left where a
            // starting window looks for it and announced to one that is already up; the
            // window takes it from `pending_open` either way, so it is opened once.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = event {
                let wanted = urls
                    .iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .find(|path| is_structure_file(path));
                if let Some(path) = wanted {
                    match app.try_state::<AppState>() {
                        Some(state) => {
                            *state
                                .pending_open
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
                            let _ = app.emit("open-file", ());
                        }
                        // The app was launched by the file: the event can come before
                        // setup has made the state, and is kept for it to pick up.
                        None => {
                            *OPENED_EARLY
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
                        }
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}

/// A structure the app was launched to open, if that was announced before there was
/// anywhere to put it; see the run loop.
static OPENED_EARLY: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Whether a path names a file this app reads, by its extension.
fn is_structure_file(path: &std::path::Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    matches!(extension.as_deref(), Some("pdb" | "cif" | "mmcif" | "ent")) && path.is_file()
}

/// Puts the main window back where it was left, if that place still exists.
fn restore_frame(app: &AppHandle, store: &Store) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Some(saved) = store.setting(window_frame::KEY).ok().flatten() else {
        return;
    };
    let Some(frame) = window_frame::parse(&saved) else {
        return;
    };
    let screens: Vec<window_frame::Screen> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| window_frame::Screen {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
            scale: monitor.scale_factor(),
        })
        .collect();
    if let Some(frame) = window_frame::fit(frame, &screens) {
        let _ = window.set_size(tauri::PhysicalSize::new(frame.width, frame.height));
        let _ = window.set_position(tauri::PhysicalPosition::new(frame.x, frame.y));
    }
}

/// Writes the main window's frame down.
///
/// A drag reports dozens of moves a second, so between writes at least half a second has
/// to pass - except for the one made as the window closes, which is the one that counts.
/// A full-screen, maximised or minimised window is not a frame to come back to.
fn remember_frame(window: &tauri::Window, closing: bool) {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);

    if window.is_fullscreen().unwrap_or(false)
        || window.is_maximized().unwrap_or(false)
        || window.is_minimized().unwrap_or(false)
    {
        return;
    }
    {
        let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !closing && last.is_some_and(|at| at.elapsed() < Duration::from_millis(500)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let frame = window_frame::Frame {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };
    let Some(state) = window.try_state::<AppState>() else {
        return;
    };
    if let Ok(text) = serde_json::to_string(&frame) {
        let _ = state.store.set_setting(window_frame::KEY, &text);
    }
}

/// Every install gets a provider for this machine, so the first run has somewhere to send
/// a job without a setup wizard. It is deliberately conservative: one job at a time.
fn seed_local_provider(store: &Store) -> Result<(), Box<dyn std::error::Error>> {
    const SEEDED: &str = "seeded_local_provider";
    if store.setting(SEEDED)?.is_some() {
        return Ok(());
    }
    let name = hostname().unwrap_or_else(|| "This machine".to_string());
    store.upsert_provider(&ProviderRecord {
        id: "local".into(),
        name,
        config: ProviderConfig::Local(LocalConfig {
            exec: ExecOptions {
                max_parallel: 1,
                ..Default::default()
            },
        }),
        enabled: true,
        created_at: util::now_iso(),
        probe: None,
        status: ProviderStatus::Unknown,
    })?;
    store.set_setting(SEEDED, &util::now_iso())?;
    Ok(())
}

/// A structure file named on the command line, so `geis model.cif` opens it - and so the
/// viewer can be driven without a mouse.
fn structure_path_from_args() -> Option<PathBuf> {
    std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .find(|path| is_structure_file(path))
}

/// A debug build loads the Vite dev server, not the bundled files. Launching the binary
/// directly therefore produces a blank window with nothing logged anywhere - the webview
/// console is not visible from the terminal. One TCP connect turns that into an
/// explanation.
fn warn_if_dev_server_is_missing(app: &AppHandle) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(url) = app.config().build.dev_url.clone() else {
        return;
    };
    let host = url.host_str().unwrap_or("localhost").to_string();
    let port = url.port_or_known_default().unwrap_or(80);

    let reachable = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port))
        .map(|addresses| {
            addresses.into_iter().any(|address| {
                std::net::TcpStream::connect_timeout(
                    &address,
                    std::time::Duration::from_millis(400),
                )
                .is_ok()
            })
        })
        .unwrap_or(false);

    if !reachable {
        eprintln!(
            "the dev server at {url} is not reachable, so this window will be blank.\n\
             run `npm run tauri dev` from the project root instead of the binary directly,\n\
             or build a release bundle with `npm run tauri build` to use the bundled files."
        );
    }
}

fn hostname() -> Option<String> {
    let output = std::process::Command::new("hostname").output().ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}
