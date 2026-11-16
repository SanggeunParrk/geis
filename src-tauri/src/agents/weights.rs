//! Local models: the catalogue, and getting one onto the machine.
//!
//! A model that runs here is a file of weights, and the file is several gigabytes. Which is
//! why the app ships the *list* and not the weights: a structure viewer that downloaded as
//! five gigabytes would be a structure viewer nobody downloads. So the panel offers a
//! short, curated choice, and the user presses a button.
//!
//! Curated rather than a search box because the choice that matters is not "which of the
//! thousands" but "which of these three will work on this machine" - and because a model
//! that cannot call a tool reliably is worse than no local model at all, since it fails by
//! doing the wrong thing rather than by saying it cannot.
//!
//! Every entry here is redistributable and quantised to GGUF, the format llama.cpp reads.
//! The licence is named because it travels with the file: Apache-2.0 asks nothing of the
//! person who downloads it, and that is why the defaults are Apache-2.0.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How well a model does the one thing this app needs: call a tool with correct arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolUse {
    Good,
    Fair,
    Poor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub id: String,
    pub name: String,
    /// One line: what it is for, and what it is not for.
    pub summary: String,
    /// The file it becomes on disk.
    pub file: String,
    pub url: String,
    /// Bytes, for the progress bar and for the "this is what you are about to download".
    pub size: u64,
    pub licence: String,
    pub tool_use: ToolUse,
    /// Whether it reasons before answering - slower, and much better at a task with steps.
    pub thinks: bool,
    /// Roughly how much memory it needs while answering.
    pub ram_gb: f32,
    /// True for the one a first-time user should press.
    pub recommended: bool,
}

/// What is offered. Short on purpose.
///
/// Sizes are the real content lengths, checked against the servers, so the panel can say
/// what a download will cost before it starts. The licence is named for each because it
/// differs: the 3B is Qwen's research licence rather than Apache-2.0, and someone using
/// this at work needs to know that without going to look it up.
pub fn catalogue() -> Vec<LocalModel> {
    vec![
        LocalModel {
            id: "qwen3.8-9b-q4".into(),
            name: "Qwen3.8 9B".into(),
            summary: "The one to take. Reasons before it answers and calls tools reliably.".into(),
            file: "qwen3.8-9b-q4_k_m.gguf".into(),
            url: "https://huggingface.co/empero-ai/Qwen3.8-9B-Distill-GGUF/resolve/main/Qwen3.8-9B-Q4_K_M.gguf".into(),
            size: 5_780_090_176,
            licence: "Apache-2.0".into(),
            tool_use: ToolUse::Good,
            thinks: true,
            ram_gb: 8.0,
            recommended: true,
        },
        LocalModel {
            id: "qwen3.8-4b-q4".into(),
            name: "Qwen3.8 4B".into(),
            summary: "The same generation in half the memory. For a laptop with 8 GB.".into(),
            file: "qwen3.8-4b-q4_k_m.gguf".into(),
            url: "https://huggingface.co/empero-ai/Qwen3.8-4B-Distill-GGUF/resolve/main/Qwen3.8-4B-Q4_K_M.gguf".into(),
            size: 2_783_446_304,
            licence: "Apache-2.0".into(),
            tool_use: ToolUse::Fair,
            thinks: true,
            ram_gb: 4.5,
            recommended: false,
        },
        LocalModel {
            id: "qwen3.8-27b-q4".into(),
            name: "Qwen3.8 27B".into(),
            summary: "The best that runs on one machine. Wants a workstation, not a laptop.".into(),
            file: "qwen3.8-27b-q4_k_m.gguf".into(),
            // The llama.cpp project's own conversion, which is the one its server is tested
            // against.
            url: "https://huggingface.co/ggml-org/Qwen3.8-27B-GGUF/resolve/main/Qwen3.8-27B-Q4_K_M.gguf".into(),
            size: 19_403_684_480,
            licence: "Apache-2.0".into(),
            tool_use: ToolUse::Good,
            thinks: true,
            ram_gb: 22.0,
            recommended: false,
        },
        LocalModel {
            id: "qwen2.5-0.5b-instruct-q4".into(),
            name: "Qwen2.5 0.5B Instruct".into(),
            summary: "Two generations old and tiny. Only for checking that the machinery runs.".into(),
            file: "qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            size: 491_400_032,
            licence: "Apache-2.0".into(),
            tool_use: ToolUse::Poor,
            thinks: false,
            ram_gb: 1.0,
            recommended: false,
        },
    ]
}

pub fn by_id(id: &str) -> Option<LocalModel> {
    catalogue().into_iter().find(|model| model.id == id)
}

/// Where weights live: beside the app's other data, not in the bundle.
pub fn weights_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("llm")
}

pub fn weights_path(data_dir: &Path, model: &LocalModel) -> PathBuf {
    weights_dir(data_dir).join(&model.file)
}

/// The half-finished file a download writes to, so a finished file is always a whole one.
pub fn part_path(data_dir: &Path, model: &LocalModel) -> PathBuf {
    weights_dir(data_dir).join(format!("{}.part", model.file))
}

/// A catalogue entry plus what is on this machine.
#[derive(Debug, Clone, Serialize)]
pub struct ModelState {
    #[serde(flatten)]
    pub model: LocalModel,
    pub installed: bool,
    /// Bytes on disk: the whole file, or how far a part-finished download got.
    pub have: u64,
    pub path: Option<String>,
}

/// Weights on the disk that the catalogue does not list any more.
///
/// A list that changes leaves files behind, and a file of five gigabytes that nothing in
/// the panel mentions is five gigabytes nobody can get back.
fn strays(data_dir: &Path, known: &[ModelState]) -> Vec<ModelState> {
    let Ok(entries) = std::fs::read_dir(weights_dir(data_dir)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            if !name.ends_with(".gguf") || known.iter().any(|state| state.model.file == name) {
                return None;
            }
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            Some(ModelState {
                installed: true,
                have: size,
                path: Some(path.to_string_lossy().to_string()),
                model: LocalModel {
                    id: format!("file:{name}"),
                    name: name.trim_end_matches(".gguf").to_string(),
                    summary: "Left over from an older list. Remove it if you do not want it."
                        .into(),
                    file: name,
                    url: String::new(),
                    size,
                    licence: String::new(),
                    tool_use: ToolUse::Fair,
                    thinks: false,
                    ram_gb: 0.0,
                    recommended: false,
                },
            })
        })
        .collect()
}

pub fn states(data_dir: &Path) -> Vec<ModelState> {
    let mut listed = states_of_catalogue(data_dir);
    listed.extend(strays(data_dir, &listed));
    listed
}

fn states_of_catalogue(data_dir: &Path) -> Vec<ModelState> {
    catalogue()
        .into_iter()
        .map(|model| {
            let full = weights_path(data_dir, &model);
            let part = part_path(data_dir, &model);
            let installed = full.exists();
            let have = if installed {
                std::fs::metadata(&full).map(|meta| meta.len()).unwrap_or(0)
            } else {
                std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0)
            };
            ModelState {
                installed,
                have,
                path: installed.then(|| full.to_string_lossy().to_string()),
                model,
            }
        })
        .collect()
}

/// How a download is going, as the panel hears about it.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub id: String,
    pub received: u64,
    pub total: u64,
    /// `downloading`, `done`, `cancelled`, or `failed`.
    pub state: String,
    pub error: Option<String>,
}

/// Fetches `url` into `part`, then renames it to `target`.
///
/// Apart from the command that calls it so it can be tested: the interesting behaviour is
/// resuming and stopping, and neither is worth finding out about for the first time four
/// gigabytes into someone's evening. `already` is what a previous attempt left behind;
/// when the server honours the range, the new bytes are appended, and when it does not -
/// it answers 200 with the whole file - the part is started again rather than appended to,
/// which would make a corrupt file that looks complete.
/// How long one wait for data lasts, and how many silent waits in a row end the download.
const STALL_SLICE: std::time::Duration = std::time::Duration::from_secs(5);
const STALL_SLICES: u32 = 18;

pub struct Fetch<'a> {
    pub client: &'a reqwest::Client,
    pub url: &'a str,
    /// Where the bytes accumulate.
    pub part: &'a Path,
    /// Where they end up, renamed, once there are all of them.
    pub target: &'a Path,
    /// What a previous attempt left in `part`.
    pub already: u64,
    /// The catalogue's size, used when the server does not say.
    pub fallback_size: u64,
    pub cancel: &'a std::sync::atomic::AtomicBool,
}

pub async fn fetch(
    job: Fetch<'_>,
    mut report: impl FnMut(u64, u64, &str, Option<String>),
) -> Result<bool, String> {
    let Fetch {
        client,
        url,
        part,
        target,
        already,
        fallback_size,
        cancel,
    } = job;
    use std::io::Write;
    use std::sync::atomic::Ordering;

    let mut request = client.get(url);
    if already > 0 {
        request = request.header("Range", format!("bytes={already}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("could not reach {url}: {error}"))?;
    if response.status().as_u16() == 416 && already > 0 {
        // "Range not satisfiable": there is nothing after what is already here. Either
        // the part is the whole file - the app was closed between the last byte and the
        // rename - or it is longer than the file now on the server. Without this, every
        // later attempt asks for the same impossible range and fails the same way.
        if already == fallback_size {
            std::fs::rename(part, target).map_err(|error| error.to_string())?;
            report(already, already, "done", None);
            return Ok(true);
        }
        std::fs::remove_file(part).ok();
        let message =
            "the partial download did not match the file on the server and was cleared - start it again"
                .to_string();
        report(0, fallback_size, "failed", Some(message.clone()));
        return Err(message);
    }
    if !response.status().is_success() {
        let message = format!("{}: {url}", response.status());
        report(already, fallback_size, "failed", Some(message.clone()));
        return Err(message);
    }
    let resuming = response.status().as_u16() == 206;
    let declared = response
        .content_length()
        .map(|length| length + if resuming { already } else { 0 });
    let total = declared.unwrap_or(fallback_size);
    let mut received = if resuming { already } else { 0 };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resuming)
        .truncate(!resuming)
        .open(part)
        .map_err(|error| {
            let message = error.to_string();
            report(received, total, "failed", Some(message.clone()));
            message
        })?;

    let mut response = response;
    let mut stalled = 0u32;
    let mut last = std::time::Instant::now();
    report(received, total, "downloading", None);
    loop {
        if cancel.load(Ordering::Relaxed) {
            report(received, total, "cancelled", None);
            return Ok(false);
        }
        // A connection that goes quiet - a closed lid, a dropped network - does not
        // error, it just stops, and waiting on it without a limit means Stop is never
        // heard and the model stays "already downloading" until the app is restarted. So
        // the wait is cut into short pieces: each one is a chance to notice a cancel, and
        // a run of silent ones is a failure, which leaves a part that resumes.
        let next = tokio::time::timeout(STALL_SLICE, response.chunk()).await;
        let chunk = match next {
            Err(_) => {
                stalled += 1;
                if stalled >= STALL_SLICES {
                    let message = format!(
                        "the download stopped receiving data for {} seconds - resume it to carry on",
                        STALL_SLICE.as_secs() * u64::from(STALL_SLICES)
                    );
                    report(received, total, "failed", Some(message.clone()));
                    return Err(message);
                }
                continue;
            }
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                let message = error.to_string();
                report(received, total, "failed", Some(message.clone()));
                return Err(message);
            }
        };
        stalled = 0;
        if let Err(error) = file.write_all(&chunk) {
            let message = error.to_string();
            report(received, total, "failed", Some(message.clone()));
            return Err(message);
        }
        received += chunk.len() as u64;
        // Often enough to feel live, rarely enough not to flood the webview.
        if last.elapsed() >= std::time::Duration::from_millis(250) {
            last = std::time::Instant::now();
            report(received, total, "downloading", None);
        }
    }
    let finish = (|| {
        file.flush().map_err(|error| error.to_string())?;
        drop(file);
        // The server said how long the file is; a stream that ended short of that is not
        // the file, and renaming it would make a truncated model look installed.
        if let Some(expected) = declared {
            if received != expected {
                return Err(format!(
                    "the download ended early ({received} of {expected} bytes) - resume it to carry on"
                ));
            }
        }
        std::fs::rename(part, target).map_err(|error| error.to_string())
    })();
    if let Err(message) = finish {
        report(received, total, "failed", Some(message.clone()));
        return Err(message);
    }
    report(received, received, "done", None);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_short_honest_and_downloadable() {
        let models = catalogue();
        assert!(models.len() <= 4, "a curated list stops being curated");
        assert_eq!(models.iter().filter(|model| model.recommended).count(), 1);
        for model in &models {
            assert!(
                model.url.starts_with("https://"),
                "{} is not https",
                model.id
            );
            assert!(model.file.ends_with(".gguf"));
            assert!(model.size > 0 && !model.licence.is_empty());
        }
        // The one that is recommended must be the one that can actually call a tool.
        let recommended = models.iter().find(|model| model.recommended).unwrap();
        assert_eq!(recommended.tool_use, ToolUse::Good);
        assert_eq!(recommended.licence, "Apache-2.0");
        // And it should be one that reasons: a task with steps is what this app asks for.
        assert!(recommended.thinks);
        assert!(by_id("qwen3.8-9b-q4").is_some());
        assert!(by_id("nope").is_none());
    }

    /// A one-request server: answers a range or the whole body, then stops.
    fn serve(body: Vec<u8>, honour_range: bool) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let from = request
                .lines()
                .find_map(|line| line.strip_prefix("Range: bytes="))
                .and_then(|value| value.trim_end_matches('-').trim().parse::<usize>().ok());
            let (status, slice) = match from {
                Some(start) if honour_range => {
                    ("206 Partial Content", &body[start.min(body.len())..])
                }
                _ => ("200 OK", &body[..]),
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                slice.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(slice).unwrap();
            stream.flush().ok();
        });
        (format!("http://127.0.0.1:{port}/model.gguf"), handle)
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("geis-weights-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_download_lands_as_one_whole_file_and_reports_as_it_goes() {
        let dir = scratch();
        let body: Vec<u8> = (0..5000u32).map(|value| value as u8).collect();
        let (url, server) = serve(body.clone(), true);
        let part = dir.join("model.part");
        let target = dir.join("model.gguf");
        let mut states = Vec::new();
        let done = fetch(
            Fetch {
                client: &reqwest::Client::new(),
                url: &url,
                part: &part,
                target: &target,
                already: 0,
                fallback_size: body.len() as u64,
                cancel: &std::sync::atomic::AtomicBool::new(false),
            },
            |received, total, state, _| states.push((received, total, state.to_string())),
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert!(done);
        // The part file is gone and the whole file is there, byte for byte.
        assert!(!part.exists());
        assert_eq!(std::fs::read(&target).unwrap(), body);
        assert_eq!(states.first().unwrap().2, "downloading");
        assert_eq!(states.last().unwrap().2, "done");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_resumed_download_appends_and_an_ignored_range_starts_again() {
        let body: Vec<u8> = (0..5000u32).map(|value| value as u8).collect();

        // The server honours the range: the 1000 bytes already there are kept.
        let dir = scratch();
        let part = dir.join("model.part");
        let target = dir.join("model.gguf");
        std::fs::write(&part, &body[..1000]).unwrap();
        let (url, server) = serve(body.clone(), true);
        fetch(
            Fetch {
                client: &reqwest::Client::new(),
                url: &url,
                part: &part,
                target: &target,
                already: 1000,
                fallback_size: body.len() as u64,
                cancel: &std::sync::atomic::AtomicBool::new(false),
            },
            |_, _, _, _| {},
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), body);

        // The server ignores it and sends everything: the part is started again rather
        // than appended to, which would have produced 6000 corrupt bytes.
        let dir2 = scratch();
        let part2 = dir2.join("model.part");
        let target2 = dir2.join("model.gguf");
        std::fs::write(&part2, &body[..1000]).unwrap();
        let (url2, server2) = serve(body.clone(), false);
        fetch(
            Fetch {
                client: &reqwest::Client::new(),
                url: &url2,
                part: &part2,
                target: &target2,
                already: 1000,
                fallback_size: body.len() as u64,
                cancel: &std::sync::atomic::AtomicBool::new(false),
            },
            |_, _, _, _| {},
        )
        .await
        .unwrap();
        server2.join().unwrap();
        assert_eq!(std::fs::read(&target2).unwrap(), body);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&dir2).ok();
    }

    #[tokio::test]
    async fn a_cancelled_download_keeps_what_it_has_and_leaves_no_whole_file() {
        let dir = scratch();
        let body: Vec<u8> = vec![7u8; 4096];
        let (url, server) = serve(body.clone(), true);
        let part = dir.join("model.part");
        let target = dir.join("model.gguf");
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let mut last = String::new();
        let done = fetch(
            Fetch {
                client: &reqwest::Client::new(),
                url: &url,
                part: &part,
                target: &target,
                already: 0,
                fallback_size: body.len() as u64,
                cancel: &cancel,
            },
            |_, _, state, _| last = state.to_string(),
        )
        .await
        .unwrap();
        server.join().ok();
        assert!(!done);
        assert_eq!(last, "cancelled");
        assert!(
            !target.exists(),
            "a cancelled download must not look finished"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_part_finished_download_is_reported_as_progress_not_as_installed() {
        let dir = std::env::temp_dir().join(format!("geis-weights-{}", uuid::Uuid::new_v4()));
        let model = by_id("qwen2.5-0.5b-instruct-q4").unwrap();
        std::fs::create_dir_all(weights_dir(&dir)).unwrap();
        std::fs::write(part_path(&dir, &model), vec![0u8; 1024]).unwrap();
        let state = states(&dir)
            .into_iter()
            .find(|s| s.model.id == model.id)
            .unwrap();
        assert!(!state.installed);
        assert_eq!(state.have, 1024);
        assert!(state.path.is_none());

        std::fs::write(weights_path(&dir, &model), vec![0u8; 2048]).unwrap();
        let state = states(&dir)
            .into_iter()
            .find(|s| s.model.id == model.id)
            .unwrap();
        assert!(state.installed);
        assert_eq!(state.have, 2048);
        assert!(state.path.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
