//! HTTPS to RetroAchievements. BaseOS ships a static `curl` with CA certificates for exactly
//! this, so slot uses it rather than carrying a TLS stack of its own.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The server answered, with this HTTP status and body.
    Answered { status: u16, body: String },
    /// No answer: no network, a timeout, or no curl at all.
    Unreachable,
}

pub trait Transport: Send + Sync {
    fn post(&self, url: &str, body: &str, content_type: &str) -> Reply;
    /// Saves `url` to `dest`. True if it arrived whole.
    fn fetch(&self, url: &str, dest: &Path) -> bool;
}

pub struct Curl {
    bin: PathBuf,
    cacert: Option<PathBuf>,
    user_agent: String,
}

const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

impl Curl {
    pub fn new(user_agent: &str) -> Curl {
        let bin = ["/usr/bin/curl", "/bin/curl"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("curl"));
        Curl {
            bin,
            cacert: Some(PathBuf::from(CA_BUNDLE)).filter(|p| p.is_file()),
            user_agent: user_agent.to_string(),
        }
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(["-sS", "-L", "--max-time", "20", "--connect-timeout", "8"])
            .arg("-A")
            .arg(&self.user_agent);
        if let Some(ca) = &self.cacert {
            c.arg("--cacert").arg(ca);
        }
        c.stderr(Stdio::null());
        c
    }
}

/// Splits curl's output into the body and the status line `-w` added after it.
pub fn split_status(out: &str) -> Option<(u16, String)> {
    let (body, code) = out.rsplit_once('\n')?;
    Some((code.trim().parse().ok()?, body.to_string()))
}

impl Transport for Curl {
    fn post(&self, url: &str, body: &str, content_type: &str) -> Reply {
        // The body goes in on stdin, so a password never shows in the process list.
        let mut c = self.command();
        if !content_type.is_empty() {
            c.arg("-H").arg(format!("Content-Type: {content_type}"));
        }
        let child = c
            .args(["--data-binary", "@-", "-w", "\n%{http_code}"])
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();
        let Ok(mut child) = child else {
            return Reply::Unreachable;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(body.as_bytes());
        }
        let Ok(out) = child.wait_with_output() else {
            return Reply::Unreachable;
        };
        if !out.status.success() {
            return Reply::Unreachable;
        }
        match split_status(&String::from_utf8_lossy(&out.stdout)) {
            Some((status, body)) if status != 0 => Reply::Answered { status, body },
            _ => Reply::Unreachable,
        }
    }

    fn fetch(&self, url: &str, dest: &Path) -> bool {
        let Some(dir) = dest.parent() else {
            return false;
        };
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
        let part = dest.with_extension("part");
        let ok = self
            .command()
            .arg("-f")
            .arg("-o")
            .arg(&part)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok && std::fs::rename(&part, dest).is_ok() {
            return true;
        }
        let _ = std::fs::remove_file(&part);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_comes_off_the_end() {
        assert_eq!(
            split_status("{\"Success\":true}\n200"),
            Some((200, "{\"Success\":true}".into()))
        );
        assert_eq!(
            split_status("line one\nline two\n404"),
            Some((404, "line one\nline two".into()))
        );
        assert_eq!(split_status("no status"), None);
    }
}
