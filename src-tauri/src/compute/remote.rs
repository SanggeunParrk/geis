//! Talking to another host through the system `ssh` and `rsync`.
//!
//! No SSH library is linked on purpose. Clusters are reached through `~/.ssh/config`
//! aliases, ProxyJump, agent forwarding, hardware tokens, and kerberos, and the user has
//! already made all of that work in their terminal. Shelling out inherits it for free;
//! an embedded client would have to reimplement each one.

use super::exec::{self, Captured};
use super::provider::ProviderError;
use crate::models::template::shell_quote;
use std::path::Path;
use std::time::Duration;

pub struct Remote {
    pub host: String,
    pub options: Vec<String>,
}

impl Remote {
    pub fn new(host: String, options: Vec<String>) -> Self {
        Self { host, options }
    }

    /// `BatchMode=yes` matters more than it looks: without it a host needing a password
    /// leaves `ssh` waiting on a prompt nobody can see, and the job hangs forever instead
    /// of failing with something actionable.
    fn base_options(&self) -> String {
        let mut parts = vec![
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ConnectTimeout=15".into(),
        ];
        parts.extend(self.options.iter().map(|o| shell_quote(o)));
        parts.join(" ")
    }

    /// `ssh <options> <host>`, ready to have a remote command appended.
    pub fn ssh_prefix(&self) -> String {
        format!("ssh {} {}", self.base_options(), shell_quote(&self.host))
    }

    /// The `-e` value rsync needs so it uses the same ssh settings, as one word for the
    /// local shell.
    ///
    /// Single-quoted rather than wrapped in double quotes: inside double quotes the local
    /// shell still expands `$` and backticks, so an ssh option such as a ProxyCommand
    /// with `$HOME` in it was expanded here, and one with a `"` ended the word early.
    /// rsync splits the value into arguments itself and honours the inner quotes.
    fn rsync_shell(&self) -> String {
        shell_quote(&format!("ssh {}", self.base_options()))
    }

    /// Run a script on the remote host and capture its output.
    pub async fn capture_script(
        &self,
        script: &str,
        timeout: Duration,
    ) -> Result<Captured, ProviderError> {
        let command = format!("{} sh -s", self.ssh_prefix());
        let captured = exec::capture(&command, Some(script), timeout).await?;
        // 255 is ssh's own "I could not connect", distinct from the remote command's
        // exit code, and worth reporting differently.
        if captured.code == 255 {
            return Err(ProviderError::Unreachable(format!(
                "ssh to {} failed: {}",
                self.host,
                captured.message()
            )));
        }
        Ok(captured)
    }

    pub async fn run_quiet(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<Captured, ProviderError> {
        self.capture_script(command, timeout).await
    }

    pub async fn mkdir(&self, remote_dir: &str) -> Result<(), ProviderError> {
        let captured = self
            .capture_script(
                &format!("mkdir -p {}", shell_quote(remote_dir)),
                Duration::from_secs(30),
            )
            .await?;
        if !captured.ok() {
            return Err(ProviderError::Remote(format!(
                "could not create {remote_dir} on {}: {}",
                self.host,
                captured.message()
            )));
        }
        Ok(())
    }

    /// Local directory -> remote directory. Trailing slashes are load-bearing in rsync;
    /// they are added here so callers cannot get it subtly wrong.
    ///
    /// `job.log` stays out of both directions. It is open and growing while this runs, so
    /// an uploaded copy is a stale few lines - and pulling that copy back replaces the
    /// real log by rename, leaving the sink writing to a file nobody can read.
    pub async fn upload(&self, local: &Path, remote_dir: &str) -> Result<(), ProviderError> {
        let local = local
            .to_str()
            .ok_or_else(|| ProviderError::Setup("local path is not valid UTF-8".into()))?;
        let command = format!(
            "rsync -az --delete-after --exclude /job.log -e {} {}/ {}:{}/",
            self.rsync_shell(),
            shell_quote(local.trim_end_matches('/')),
            shell_quote(&self.host),
            shell_quote(remote_dir.trim_end_matches('/'))
        );
        self.rsync(&command, "upload").await
    }

    /// Remote results back to the local workdir. `.geis/` holds the scripts we generated
    /// and uploaded; pulling them back would be pointless churn.
    pub async fn download(&self, remote_dir: &str, local: &Path) -> Result<(), ProviderError> {
        let local = local
            .to_str()
            .ok_or_else(|| ProviderError::Setup("local path is not valid UTF-8".into()))?;
        let command = format!(
            "rsync -az --exclude .geis/ --exclude .mini/ --exclude /job.log -e {} {}:{}/ {}/",
            self.rsync_shell(),
            shell_quote(&self.host),
            shell_quote(remote_dir.trim_end_matches('/')),
            shell_quote(local.trim_end_matches('/'))
        );
        self.rsync(&command, "download").await
    }

    async fn rsync(&self, command: &str, direction: &str) -> Result<(), ProviderError> {
        let captured = exec::capture(command, None, Duration::from_secs(3600)).await?;
        if !captured.ok() {
            return Err(ProviderError::Remote(format!(
                "rsync {direction} failed: {}",
                captured.message()
            )));
        }
        Ok(())
    }

    /// Read a byte range of a remote file, for tailing a scheduler's output file.
    /// `offset` is 0-based; `tail -c +N` is 1-based, hence the +1.
    ///
    /// Returns the text and how many bytes of the file it stands for, which is what the
    /// caller must advance its offset by - see `decode_tail`.
    pub async fn tail_from(
        &self,
        remote_path: &str,
        offset: u64,
    ) -> Result<(String, u64), ProviderError> {
        let command = format!(
            "tail -c +{} {} 2>/dev/null || true",
            offset + 1,
            shell_quote(remote_path)
        );
        let captured = self
            .capture_script(&command, Duration::from_secs(60))
            .await?;
        Ok(decode_tail(&captured.stdout_raw))
    }
}

/// A piece of a growing file as text, and how many of its bytes that text accounts for.
///
/// The byte count is not the text's length. A character the job was half-way through
/// writing is left for the next read rather than shown as a replacement mark and then
/// skipped, and invalid bytes that become three-byte replacement marks still advance the
/// offset by one each - counting the text instead skips output the job wrote after them.
pub fn decode_tail(bytes: &[u8]) -> (String, u64) {
    let complete = crate::util::utf8_complete_len(bytes);
    (
        String::from_utf8_lossy(&bytes[..complete]).into_owned(),
        complete as u64,
    )
}

/// Where a job's files live on the remote host: one directory per job, named after the
/// local workdir so the two are trivially matched up when debugging.
pub fn remote_workdir(remote_root: &str, local_dir: &Path) -> String {
    let leaf = local_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("job");
    format!("{}/{}", remote_root.trim_end_matches('/'), leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_never_skips_or_mangles_bytes() {
        // A clean read is all text and all bytes.
        assert_eq!(decode_tail(b"step 1\n"), ("step 1\n".to_string(), 7));
        // A character cut by the read is left for the next one, and the offset stops
        // before it, so the next read starts at its first byte.
        let text = "진행 50%".as_bytes();
        let (shown, consumed) = decode_tail(&text[..4]);
        assert_eq!((shown.as_str(), consumed), ("진", 3));
        let (rest, _) = decode_tail(&text[consumed as usize..]);
        assert_eq!(format!("{shown}{rest}"), "진행 50%");
        // An invalid byte counts as the one byte it is, not the three of its replacement.
        let (shown, consumed) = decode_tail(b"a\xffb");
        assert_eq!(consumed, 3);
        assert_eq!(shown, "a\u{fffd}b");
    }

    #[test]
    fn the_rsync_shell_is_one_word_the_local_shell_leaves_alone() {
        let remote = Remote::new(
            "gpu".into(),
            vec!["-o".into(), "ProxyCommand=ssh -W %h:%p $JUMP \"x\"".into()],
        );
        let word = remote.rsync_shell();
        // Single-quoted as a whole: no `$` or `"` is left where the local shell acts on it.
        assert!(word.starts_with('\'') && word.ends_with('\''), "{word}");
        assert!(word.contains("$JUMP"));
        assert!(word.contains("BatchMode=yes"));
    }

    #[test]
    fn the_remote_workdir_is_named_after_the_local_one() {
        assert_eq!(
            remote_workdir("/scratch/me/", Path::new("/Users/me/jobs/fold-abc12345")),
            "/scratch/me/fold-abc12345"
        );
        assert_eq!(remote_workdir("/scratch", Path::new("/")), "/scratch/job");
    }
}
