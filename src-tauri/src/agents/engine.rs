//! The engine that runs local weights.
//!
//! Downloading a model is only half of "no installing anything": the weights are data, and
//! something has to run them. That something is llama.cpp's server, which speaks the same
//! OpenAI-compatible API the `local` connection already talks to - so once it is running,
//! nothing above this module knows the difference between it and Ollama.
//!
//! It is fetched rather than shipped in the bundle, for two reasons. It is another
//! project's executable, and putting one of those inside a signed and notarised app makes
//! the signature ours to defend; and a build per platform would have to be produced by our
//! own build, on machines we do not have. Fetching it pins an exact llama.cpp build and
//! checks the SHA-256 that GitHub publishes for that file, so what runs is what was meant,
//! and an app that cannot verify it refuses to install it.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// The llama.cpp release this app was written against.
///
/// Pinned, not "latest": a new build could change the flags, the layout of the archive, or
/// the shape of the API, and the first anyone would know is a user whose panel stopped
/// answering.
pub const BUILD: &str = "b11093";

/// One platform's archive, with the digest GitHub publishes for it.
#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

const ASSETS: &[(&str, Asset)] = &[
    (
        "macos-arm64",
        Asset {
            name: "llama-b11093-bin-macos-arm64.tar.gz",
            sha256: "2cb3a819b07799dd91c6270d53fafb2a509285218d81d688b0ea3b4d6d3319bc",
            size: 11_195_997,
        },
    ),
    (
        "macos-x64",
        Asset {
            name: "llama-b11093-bin-macos-x64.tar.gz",
            sha256: "03d0405f3e75dbd145aff58a1db0874a7cd73e8cff41e6d56511145357d52ac2",
            size: 11_226_012,
        },
    ),
    (
        "linux-arm64",
        Asset {
            name: "llama-b11093-bin-ubuntu-arm64.tar.gz",
            sha256: "69f2fe691bab1230ea463f2240a7a268c7d45dd0ad26d49282e3cced43f5e75a",
            size: 13_584_600,
        },
    ),
    (
        "linux-x64",
        Asset {
            name: "llama-b11093-bin-ubuntu-x64.tar.gz",
            sha256: "95c4a9f486b4cd690f29f560c9805c5fe6ba799f043186fd8f0a1bd55938ff73",
            size: 16_983_817,
        },
    ),
    (
        "windows-x64",
        Asset {
            name: "llama-b11093-bin-win-cpu-x64.zip",
            sha256: "eb653e11efd2eb0c2247e9bde5dd9d21a3e7755c1ad321589944aa800615e50d",
            size: 18_550_781,
        },
    ),
];

/// Which archive this machine needs, by the platform the binary was compiled for.
pub fn target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "windows-x64",
        _ => return None,
    })
}

pub fn asset() -> Option<&'static Asset> {
    let wanted = target()?;
    ASSETS
        .iter()
        .find(|(name, _)| *name == wanted)
        .map(|(_, asset)| asset)
}

pub fn url(asset: &Asset) -> String {
    format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{BUILD}/{}",
        asset.name
    )
}

/// Everything about the engine lives under one directory named for the build, so a later
/// version installs beside this one rather than half over it.
pub fn engine_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("engine").join(BUILD)
}

pub fn archive_path(data_dir: &Path, asset: &Asset) -> PathBuf {
    engine_dir(data_dir).join(asset.name)
}

fn server_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Where the server ended up. The archives put it under `build/bin`, but that has changed
/// before, so it is looked for rather than assumed.
pub fn server_path(data_dir: &Path) -> Option<PathBuf> {
    let root = engine_dir(data_dir);
    let direct = root.join("build").join("bin").join(server_name());
    if direct.exists() {
        return Some(direct);
    }
    walk_for(&root, server_name(), 4)
}

fn walk_for(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            return Some(path);
        }
    }
    directories
        .into_iter()
        .find_map(|path| walk_for(&path, name, depth - 1))
}

/// Where the id of the running server is written, so a later run can clean up after a
/// crash or a kill that never gave the app a chance to stop it.
pub fn pid_path(data_dir: &Path) -> PathBuf {
    engine_dir(data_dir).join("engine.pid")
}

/// Kills a server left behind by a previous run.
///
/// A child does not die with its parent, and an app that was force-quit - or crashed -
/// leaves several gigabytes of model resident with nothing to talk to it. The id alone is
/// not enough to act on, since ids are reused, so the process is only killed when it is
/// still one of ours.
pub fn kill_stale(data_dir: &Path) {
    let path = pid_path(data_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    std::fs::remove_file(&path).ok();
    let Ok(pid) = text.trim().parse::<u32>() else {
        return;
    };
    #[cfg(unix)]
    {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
        else {
            return;
        };
        let command = String::from_utf8_lossy(&output.stdout);
        if !command.contains("llama-server") {
            return;
        }
        std::process::Command::new("kill")
            .arg(pid.to_string())
            .status()
            .ok();
    }
    #[cfg(windows)]
    {
        std::process::Command::new("taskkill")
            .args([
                "/PID",
                &pid.to_string(),
                "/F",
                "/FI",
                "IMAGENAME eq llama-server.exe",
            ])
            .status()
            .ok();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineState {
    /// False on a platform we have no build for, which the panel must say rather than
    /// offering a button that cannot work.
    pub supported: bool,
    pub build: String,
    pub installed: bool,
    pub path: Option<String>,
    /// What the download will cost.
    pub size: u64,
    pub running: bool,
    pub port: Option<u16>,
    /// The weights it was started with.
    pub model: Option<String>,
    /// The window it was started with, in tokens; 0 when it has not been started.
    pub context: u32,
}

pub fn state(data_dir: &Path, running: Option<(u16, String)>) -> EngineState {
    let path = server_path(data_dir);
    EngineState {
        supported: asset().is_some(),
        build: BUILD.to_string(),
        installed: path.is_some(),
        path: path.map(|path| path.to_string_lossy().to_string()),
        size: asset().map(|asset| asset.size).unwrap_or(0),
        running: running.is_some(),
        port: running.as_ref().map(|(port, _)| *port),
        model: running.map(|(_, model)| model),
        context: started_context(),
    }
}

/// The SHA-256 of a file, as hex.
pub fn digest(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Unpacks the archive with the system `tar`, which reads both .tar.gz and .zip.
///
/// Shelled out rather than linked: an archive format is exactly the sort of thing every
/// operating system already has a correct implementation of, and two more crates to keep
/// current is a cost with no return.
pub async fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
    let output = tokio::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .output()
        .await
        .map_err(|error| format!("could not run tar: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not unpack the engine: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    Ok(())
}

/// Makes the server executable, which an archive's permissions usually but not always keep.
pub fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        std::fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// A free port, found by letting the operating system pick one and then letting it go.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .unwrap_or(18_435)
}

/// The windows a local model may be started with, widest first.
const CONTEXT_STEPS: [u32; 5] = [32_768, 24_576, 16_384, 12_288, 8_192];
/// The least that is any use: the instructions and tools given a local model, a reply,
/// and a few exchanges.
const CONTEXT_FLOOR: u32 = 8_192;
/// How much of the machine's memory the model and its window may take between them. The
/// rest is the app's own, the viewport's, and everything else that is open.
const MEMORY_SHARE: f64 = 0.55;

/// The window the engine was last started with, for the panel to budget against.
static STARTED_WITH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn started_context() -> u32 {
    STARTED_WITH.load(std::sync::atomic::Ordering::Relaxed)
}

/// What the attention cache costs per token of window, and how long a window the model
/// was trained on - read out of the weights file's own header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Bytes of key-value cache for one token: every layer keeps a key and a value for it,
    /// each `embedding / heads * kv_heads` half-precision floats.
    pub bytes_per_token: u64,
    pub trained_context: u32,
}

/// Reads a GGUF file's metadata far enough to know the model's shape.
pub fn shape(weights: &Path) -> Option<Shape> {
    use std::io::Read;
    let mut file = std::io::BufReader::new(std::fs::File::open(weights).ok()?);
    let u32_of = |file: &mut std::io::BufReader<std::fs::File>| -> Option<u32> {
        let mut bytes = [0u8; 4];
        file.read_exact(&mut bytes).ok()?;
        Some(u32::from_le_bytes(bytes))
    };
    let u64_of = |file: &mut std::io::BufReader<std::fs::File>| -> Option<u64> {
        let mut bytes = [0u8; 8];
        file.read_exact(&mut bytes).ok()?;
        Some(u64::from_le_bytes(bytes))
    };
    let text = |file: &mut std::io::BufReader<std::fs::File>| -> Option<String> {
        let length = u64_of(file)?;
        // A key or a name, not a megabyte: a header that says otherwise is not one.
        if length > 1 << 20 {
            return None;
        }
        let mut bytes = vec![0u8; length as usize];
        file.read_exact(&mut bytes).ok()?;
        Some(String::from_utf8_lossy(&bytes).to_string())
    };
    let width = |kind: u32| -> Option<u64> {
        Some(match kind {
            0 | 1 | 7 => 1,
            2 | 3 => 2,
            4..=6 => 4,
            10..=12 => 8,
            _ => return None,
        })
    };
    // A skip that runs out of file is a header that lies about its lengths. Taken at its
    // word it reads on through gigabytes of tensors looking for keys that are not there.
    let skip = |file: &mut std::io::BufReader<std::fs::File>, bytes: u64| -> Option<()> {
        let skipped = std::io::copy(&mut file.by_ref().take(bytes), &mut std::io::sink()).ok()?;
        (skipped == bytes).then_some(())
    };

    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let _version = u32_of(&mut file)?;
    let _tensors = u64_of(&mut file)?;
    let pairs = u64_of(&mut file)?;
    let mut numbers: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut architecture = String::new();
    for _ in 0..pairs.min(4096) {
        let key = text(&mut file)?;
        let kind = u32_of(&mut file)?;
        match kind {
            8 => {
                let value = text(&mut file)?;
                if key == "general.architecture" {
                    architecture = value;
                }
            }
            9 => {
                let element = u32_of(&mut file)?;
                let count = u64_of(&mut file)?;
                if element == 8 {
                    for _ in 0..count {
                        let length = u64_of(&mut file)?;
                        skip(&mut file, length)?;
                    }
                } else {
                    // Overflow is a corrupt header, not a panic.
                    skip(&mut file, width(element)?.checked_mul(count)?)?;
                }
            }
            4 => {
                numbers.insert(key, u32_of(&mut file)? as u64);
            }
            10 => {
                numbers.insert(key, u64_of(&mut file)?);
            }
            other => skip(&mut file, width(other)?)?,
        }
    }
    let of = |name: &str| numbers.get(&format!("{architecture}.{name}")).copied();
    let (layers, embedding, heads) = (
        of("block_count")?,
        of("embedding_length")?,
        of("attention.head_count")?,
    );
    let kv_heads = of("attention.head_count_kv").unwrap_or(heads);
    if heads == 0 {
        return None;
    }
    // Every factor comes from the file, so the product may not fit; a model that big is
    // not one this is going to run, and a wrapped number would make it look tiny.
    let bytes_per_token = (embedding / heads)
        .checked_mul(kv_heads)?
        .checked_mul(layers)?
        .checked_mul(4)?;
    Some(Shape {
        bytes_per_token,
        trained_context: of("context_length")
            .unwrap_or(CONTEXT_FLOOR as u64)
            .min(u32::MAX as u64) as u32,
    })
}

/// This machine's memory, in bytes. Eight gigabytes where it cannot be asked.
pub fn machine_memory() -> u64 {
    let asked = if cfg!(target_os = "macos") {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
    } else {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find(|line| line.starts_with("MemTotal:"))
                    .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                    .map(|kilobytes| kilobytes * 1024)
            })
    };
    asked.unwrap_or(8 << 30)
}

/// The widest window these weights can be given on a machine with this much memory.
///
/// A window is not free: every token of it is a key and a value kept for every layer, and
/// that cache is allocated when the engine starts, whether or not a conversation fills it.
/// For a nine-billion-parameter model it is about 135 kilobytes a token - 4.4 GB at
/// 32,768 - on top of 5.8 GB of weights, which on a sixteen-gigabyte machine leaves the
/// rest of the system nothing. So the window is the widest step at which weights and cache
/// together stay inside a share of the machine's memory, and never past what the model
/// was trained to read.
pub fn context_for(weights_bytes: u64, shape: Option<Shape>, memory: u64) -> u32 {
    let Some(shape) = shape else {
        // A file whose shape could not be read: the floor, which any model can bear.
        return CONTEXT_FLOOR;
    };
    let room = (memory as f64 * MEMORY_SHARE) as u64;
    CONTEXT_STEPS
        .iter()
        .copied()
        .find(|&tokens| {
            tokens <= shape.trained_context.max(CONTEXT_FLOOR)
                && shape
                    .bytes_per_token
                    .checked_mul(tokens as u64)
                    .and_then(|cache| cache.checked_add(weights_bytes))
                    .is_some_and(|needed| needed <= room)
        })
        .unwrap_or(CONTEXT_FLOOR)
}

/// The window for these weights on this machine, noted for the panel.
pub fn choose_context(weights: &Path) -> u32 {
    let size = std::fs::metadata(weights)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let tokens = context_for(size, shape(weights), machine_memory());
    STARTED_WITH.store(tokens, std::sync::atomic::Ordering::Relaxed);
    tokens
}

/// The arguments the server is started with.
///
/// `--jinja` is the one that matters: without it the server ignores the model's chat
/// template and cannot emit tool calls at all, which would make a local model useless for
/// everything this app asks of it.
pub fn arguments(weights: &Path, port: u16, context: u32) -> Vec<String> {
    vec![
        "-m".into(),
        weights.to_string_lossy().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        context.to_string(),
        "--jinja".into(),
        "-ngl".into(),
        // Everything on the GPU where there is one; llama.cpp ignores this on a CPU build.
        "99".into(),
    ]
}

/// Waits for the server to answer, which for a large model is tens of seconds of loading.
pub async fn wait_until_ready(port: u16, patience: std::time::Duration) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let until = std::time::Instant::now() + patience;
    let url = format!("http://127.0.0.1:{port}/health");
    loop {
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        if std::time::Instant::now() >= until {
            return Err("the engine did not come up in time".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
}

/// Where the `local` connection should point once the engine is up.
pub fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_has_a_pinned_build_with_a_digest() {
        // Every platform we claim to support must have an asset, and every asset a digest.
        for (name, asset) in ASSETS {
            assert_eq!(asset.sha256.len(), 64, "{name} has no sha256");
            assert!(asset.name.contains(BUILD), "{name} is not the pinned build");
            assert!(asset.size > 1_000_000);
            assert!(
                url(asset).starts_with("https://github.com/ggml-org/llama.cpp/releases/download/")
            );
        }
        // And the machine running the tests is one of them.
        assert!(target().is_some());
        assert!(asset().is_some());
    }

    #[test]
    fn the_server_is_started_with_a_chat_template() {
        let args = arguments(Path::new("/tmp/model.gguf"), 1234, 4096);
        assert!(
            args.contains(&"--jinja".to_string()),
            "without --jinja it cannot call tools"
        );
        assert!(args.contains(&"1234".to_string()));
        assert!(args.contains(&"/tmp/model.gguf".to_string()));
        assert_eq!(endpoint(1234), "http://127.0.0.1:1234/v1");
    }

    #[test]
    fn the_server_is_found_wherever_the_archive_put_it() {
        let dir = std::env::temp_dir().join(format!("geis-engine-{}", uuid::Uuid::new_v4()));
        let nested = engine_dir(&dir).join("build").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(server_path(&dir).is_none());
        std::fs::write(nested.join(server_name()), b"#!/bin/sh\n").unwrap();
        assert_eq!(server_path(&dir).unwrap(), nested.join(server_name()));

        // And when a later archive moves it, it is still found.
        let other = engine_dir(&dir).join("somewhere").join("else");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::remove_file(nested.join(server_name())).unwrap();
        std::fs::write(other.join(server_name()), b"#!/bin/sh\n").unwrap();
        assert_eq!(server_path(&dir).unwrap(), other.join(server_name()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_digest_is_what_the_file_actually_is() {
        let dir = std::env::temp_dir().join(format!("geis-digest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            digest(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_window_is_what_the_machine_can_carry() {
        // Qwen3.8 9B as its file describes it: 33 layers, 4096 wide, 16 heads of which 4
        // carry keys and values - 135,168 bytes a token.
        let nine = Shape {
            bytes_per_token: 33 * (4096 / 16 * 4) * 2 * 2,
            trained_context: 262_144,
        };
        assert_eq!(nine.bytes_per_token, 135_168);
        let weights = 5_780_090_176;
        // Sixteen gigabytes: 32,768 would be 4.4 GB of cache on 5.8 of weights. Not that.
        assert_eq!(context_for(weights, Some(nine), 16 << 30), 24_576);
        // Thirty-two has room for all of it; eight has room for the floor and no more.
        assert_eq!(context_for(weights, Some(nine), 32 << 30), 32_768);
        assert_eq!(context_for(weights, Some(nine), 8 << 30), CONTEXT_FLOOR);
        // Never past what the model was trained on, however much memory there is.
        let short = Shape {
            bytes_per_token: 1000,
            trained_context: 16_384,
        };
        assert_eq!(context_for(1 << 30, Some(short), 64 << 30), 16_384);
        // And a file that could not be read gets the floor.
        assert_eq!(context_for(weights, None, 64 << 30), CONTEXT_FLOOR);
    }

    /// A GGUF header written by hand: little-endian, as the format is.
    struct Gguf(Vec<u8>);

    impl Gguf {
        fn new(pairs: u64) -> Self {
            let mut bytes = b"GGUF".to_vec();
            bytes.extend(3u32.to_le_bytes());
            bytes.extend(0u64.to_le_bytes());
            bytes.extend(pairs.to_le_bytes());
            Gguf(bytes)
        }
        fn text(mut self, text: &str) -> Self {
            self.0.extend((text.len() as u64).to_le_bytes());
            self.0.extend(text.as_bytes());
            self
        }
        fn u32(mut self, value: u32) -> Self {
            self.0.extend(value.to_le_bytes());
            self
        }
        fn u64(mut self, value: u64) -> Self {
            self.0.extend(value.to_le_bytes());
            self
        }
        fn number(self, key: &str, value: u32) -> Self {
            self.text(key).u32(4).u32(value)
        }
        fn read(&self) -> Option<Shape> {
            let dir = std::env::temp_dir().join(format!("geis-gguf-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("m.gguf");
            std::fs::write(&path, &self.0).unwrap();
            let shape = shape(&path);
            std::fs::remove_dir_all(&dir).ok();
            shape
        }
    }

    fn llama(pairs_extra: u64) -> Gguf {
        Gguf::new(5 + pairs_extra)
            .text("general.architecture")
            .u32(8)
            .text("llama")
            .number("llama.block_count", 32)
            .number("llama.embedding_length", 4096)
            .number("llama.attention.head_count", 32)
            .number("llama.attention.head_count_kv", 8)
    }

    #[test]
    fn a_hand_written_header_is_read() {
        let shape = llama(0).read().unwrap();
        assert_eq!(shape.bytes_per_token, 32 * (4096 / 32 * 8) * 4);
        assert_eq!(
            shape.trained_context, CONTEXT_FLOOR,
            "no context_length: the floor"
        );
        // Arrays and other kinds of value in between are stepped over.
        let shape = llama(3)
            .text("tokenizer.ggml.tokens")
            .u32(9)
            .u32(8)
            .u64(2)
            .text("a")
            .text("bc")
            .text("tokenizer.scores")
            .u32(9)
            .u32(6)
            .u64(3)
            .u32(0)
            .u32(0)
            .u32(0)
            .text("llama.context_length")
            .u32(10)
            .u64(131_072)
            .read()
            .unwrap();
        assert_eq!(shape.trained_context, 131_072);
    }

    #[test]
    fn a_truncated_or_corrupt_header_reads_as_nothing() {
        let whole = llama(0).0;
        for cut in [0, 3, 4, 10, 24, 30, whole.len() - 1] {
            assert!(Gguf(whole[..cut].to_vec()).read().is_none(), "cut at {cut}");
        }
        assert!(Gguf(b"GGML and then some".to_vec()).read().is_none());
        // Zero heads would divide by zero.
        let zero = Gguf::new(4)
            .text("general.architecture")
            .u32(8)
            .text("llama")
            .number("llama.block_count", 32)
            .number("llama.embedding_length", 4096)
            .number("llama.attention.head_count", 0);
        assert!(zero.read().is_none());
        // An unknown value kind cannot be stepped over.
        assert!(Gguf::new(1).text("x").u32(99).read().is_none());
    }

    #[test]
    fn absurd_sizes_neither_allocate_nor_panic() {
        // A key claiming to be an exabyte long.
        assert!(Gguf::new(1).u64(u64::MAX).read().is_none());
        // An array whose length times its width overflows.
        assert!(llama(1)
            .text("a")
            .u32(9)
            .u32(12)
            .u64(u64::MAX / 2)
            .read()
            .is_none());
        // An array of strings longer than the file.
        assert!(llama(1)
            .text("a")
            .u32(9)
            .u32(8)
            .u64(u64::MAX)
            .u64(5)
            .read()
            .is_none());
        // A billion key-value pairs, of which the file holds none.
        assert!(Gguf::new(1 << 30).read().is_none());
        // Dimensions whose product does not fit in 64 bits.
        let huge = Gguf::new(5)
            .text("general.architecture")
            .u32(8)
            .text("llama")
            .text("llama.block_count")
            .u32(10)
            .u64(u64::MAX / 2)
            .text("llama.embedding_length")
            .u32(10)
            .u64(u64::MAX / 2)
            .number("llama.attention.head_count", 1)
            .number("llama.attention.head_count_kv", 8);
        assert!(huge.read().is_none());
        // And a shape at the edge still yields a window rather than an overflow.
        let edge = Shape {
            bytes_per_token: u64::MAX / 2,
            trained_context: u32::MAX,
        };
        assert_eq!(
            context_for(u64::MAX - 1, Some(edge), u64::MAX),
            CONTEXT_FLOOR
        );
    }

    /// Against a real weights file, when one is pointed at: `GEIS_TEST_GGUF=/path/to.gguf`.
    #[test]
    fn a_real_header_is_read() {
        let Some(path) = std::env::var_os("GEIS_TEST_GGUF") else {
            return;
        };
        let shape = shape(Path::new(&path)).expect("the header reads");
        eprintln!(
            "{shape:?} -> {} tokens here",
            choose_context(Path::new(&path))
        );
        assert!(shape.bytes_per_token > 1000 && shape.trained_context >= 2048);
    }
}
