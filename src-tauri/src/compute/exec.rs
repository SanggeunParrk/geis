//! Command execution over a pty, with live output capture.
//!
//! Everything runs through a pty rather than piped stdio for two reasons. Scientific
//! tooling checks `isatty` and drops its progress bars, colours, and sometimes its
//! per-step logging entirely when it thinks nothing is watching - exactly the output
//! someone needs when a 40-minute prediction is misbehaving. And the terminal panel in
//! Settings -> Compute is then a faithful view of the real thing, not a reconstruction.
//!
//! Output is forwarded as raw chunks, not lines, so carriage-return progress bars animate
//! in the UI the way they do in a terminal.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;

/// One chunk of output from a job, on its way to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct LogChunk {
    pub job_id: String,
    pub text: String,
}

/// Fans output out to the log file on disk and to whoever is watching live.
///
/// The file write is the authoritative record: a user can close the terminal panel, close
/// the whole app, and still have the log. The channel is best-effort.
#[derive(Clone)]
pub struct LogSink {
    job_id: String,
    file: Arc<Mutex<std::fs::File>>,
    tx: Option<UnboundedSender<LogChunk>>,
    /// The start of a character the last read cut off, held until the rest arrives so the
    /// live view does not show it as two replacement marks. Shared between clones: they
    /// are one stream.
    pending: Arc<Mutex<Vec<u8>>>,
}

impl LogSink {
    pub fn create(
        job_id: &str,
        path: &Path,
        tx: Option<UnboundedSender<LogChunk>>,
    ) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            job_id: job_id.to_string(),
            file: Arc::new(Mutex::new(file)),
            tx,
            pending: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(bytes);
        }
        if let Some(tx) = &self.tx {
            let text = match self.pending.lock() {
                Ok(mut pending) => decode_held(&mut pending, bytes),
                Err(_) => String::from_utf8_lossy(bytes).into_owned(),
            };
            if !text.is_empty() {
                let _ = tx.send(LogChunk {
                    job_id: self.job_id.clone(),
                    text,
                });
            }
        }
    }

    /// A line from the app itself rather than the child process. Marked so it is
    /// distinguishable from tool output in the terminal panel.
    pub fn note(&self, text: &str) {
        self.write_bytes(format!("\x1b[2m# {text}\x1b[0m\r\n").as_bytes());
    }

    pub fn error(&self, text: &str) {
        self.write_bytes(format!("\x1b[31m# {text}\x1b[0m\r\n").as_bytes());
    }
}

/// Decodes what can be decoded of `held` followed by `bytes`, keeping back a character cut
/// off at the end for next time.
fn decode_held(held: &mut Vec<u8>, bytes: &[u8]) -> String {
    held.extend_from_slice(bytes);
    let complete = crate::util::utf8_complete_len(held);
    let text = String::from_utf8_lossy(&held[..complete]).into_owned();
    held.drain(..complete);
    text
}

/// Cooperative cancellation shared between the UI ("stop this job") and the runner.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("spawning shell: {0}")]
    Spawn(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

pub struct ExecRequest {
    /// A shell command line. Run steps are shell strings on purpose: clusters need
    /// `module load`, `conda activate`, pipes, and `&&`.
    pub command: String,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

/// Runs `request.command` and returns its exit status.
///
/// The command is handed to the user's login shell so that whatever makes tools work in
/// their terminal - conda init, module systems, PATH edits in .zprofile - works here too.
/// A GUI app's inherited environment is otherwise almost never the one the user tested in.
pub async fn run(
    request: ExecRequest,
    sink: LogSink,
    cancel: CancelToken,
) -> Result<i32, ExecError> {
    let shell = login_shell();
    sink.note(&format!("{} $ {}", request.cwd.display(), request.command));

    let handle = tokio::task::spawn_blocking(move || -> Result<i32, ExecError> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ExecError::Spawn(e.to_string()))?;

        let mut builder = CommandBuilder::new(&shell.program);
        for arg in &shell.args {
            builder.arg(arg);
        }
        builder.arg("-c");
        builder.arg(&request.command);
        builder.cwd(&request.cwd);
        // Tools that ask "am I interactive?" get a sensible answer, and dumb terminals
        // avoid emitting cursor addressing the panel would have to interpret.
        builder.env("TERM", "xterm-256color");
        for (key, value) in &request.env {
            builder.env(key, value);
        }

        let mut child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| ExecError::Spawn(e.to_string()))?;
        // The slave side must be dropped or the reader never sees EOF.
        drop(pair.slave);

        // The watcher ends when the child does, not only on cancel. Left waiting for a
        // cancel that never comes it is a thread per step for the life of the app - and
        // one that, when a later step is cancelled, signals this step's long-reaped pid,
        // which by then may belong to something else.
        let mut killer = child.clone_killer();
        let cancel_watch = cancel.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let finished_watch = Arc::clone(&finished);
        std::thread::spawn(move || {
            while !cancel_watch.is_cancelled() {
                if finished_watch.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if !finished_watch.load(Ordering::SeqCst) {
                let _ = killer.kill();
            }
        });

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| ExecError::Spawn(e.to_string()))?;
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => sink.write_bytes(&buffer[..n]),
                // The pty read fails with EIO on some platforms when the child exits,
                // which is a normal end of stream, not an error worth surfacing.
                Err(_) => break,
            }
        }

        let status = child.wait();
        finished.store(true, Ordering::SeqCst);
        let status = status.map_err(ExecError::Io)?;
        if cancel.is_cancelled() {
            return Err(ExecError::Cancelled);
        }
        Ok(status.exit_code() as i32)
    });

    match handle.await {
        Ok(result) => result,
        Err(join_error) => Err(ExecError::Spawn(join_error.to_string())),
    }
}

/// Runs a command and captures its output as a string, without a pty.
///
/// For short control-plane calls - probes, `squeue`, `rsync` - where we want the text, not
/// a live view, and where terminal escapes would only be noise.
pub async fn capture(
    command: &str,
    stdin: Option<&str>,
    timeout: std::time::Duration,
) -> Result<Captured, ExecError> {
    let shell = login_shell();
    let mut child = tokio::process::Command::new(&shell.program)
        .args(&shell.args)
        .arg("-c")
        .arg(command)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // A timeout drops the wait below, and a dropped child is otherwise left running:
        // an rsync that "timed out" would go on writing into the workdir of a job already
        // reported as failed.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| ExecError::Spawn(e.to_string()))?;

    if let Some(text) = stdin {
        use tokio::io::AsyncWriteExt;
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(text.as_bytes()).await;
            let _ = pipe.shutdown().await;
        }
    }

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result.map_err(ExecError::Io)?,
        Err(_) => {
            return Err(ExecError::Spawn(format!(
                "timed out after {}s",
                timeout.as_secs()
            )))
        }
    };

    Ok(Captured {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stdout_raw: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[derive(Debug, Clone)]
pub struct Captured {
    pub code: i32,
    pub stdout: String,
    /// stdout exactly as written. `stdout` has invalid UTF-8 replaced, which changes its
    /// length, so a reader keeping a byte offset into a remote file needs these.
    pub stdout_raw: Vec<u8>,
    pub stderr: String,
}

impl Captured {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// stderr if there is any, else stdout - whichever is likelier to explain a failure.
    pub fn message(&self) -> String {
        let stderr = self.stderr.trim();
        if stderr.is_empty() {
            self.stdout.trim().lines().last().unwrap_or("").to_string()
        } else {
            stderr.lines().last().unwrap_or(stderr).to_string()
        }
    }
}

struct Shell {
    program: String,
    args: Vec<&'static str>,
}

/// The shell to run commands through.
///
/// `-l` is what makes conda init, module systems, and PATH edits in .zprofile apply, so a
/// command that works in the user's terminal works here. It is only passed to shells that
/// implement login mode usefully - `sh -i -c` in particular would enable job control and
/// emit process-management noise into the log for no benefit.
fn login_shell() -> Shell {
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let login = matches!(
        Path::new(&program).file_name().and_then(|n| n.to_str()),
        Some("zsh") | Some("bash") | Some("fish")
    );
    Shell {
        program,
        args: if login { vec!["-l"] } else { vec![] },
    }
}
