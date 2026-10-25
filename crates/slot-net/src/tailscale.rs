//! Tailscale: `tailscaled` on the card, its state kept in `Config/tailscale` so the device
//! keeps its identity, and the `tailscale` command for logging in and reading status.
//!
//! BaseOS's kernel has TUN built in, so the daemon makes a real `tailscale0` interface and
//! everything on the device, Syncthing included, can reach the tailnet. Without
//! `/dev/net/tun` it falls back to userspace networking, where the device is still reachable.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::json::Json;

/// What `tailscale status --json` says, cut down to what slot shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailPoll {
    /// NoState, NeedsLogin, NeedsMachineAuth, Stopped, Starting or Running.
    pub backend: String,
    pub auth_url: Option<String>,
    pub ip: Option<String>,
    /// This device's name on the tailnet.
    pub name: Option<String>,
    pub tailnet: Option<String>,
}

impl TailPoll {
    pub fn parse(text: &str) -> Option<TailPoll> {
        let j = Json::parse(text)?;
        let str_of = |v: &Json| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let me = j.get("Self");
        let ip = me
            .get("TailscaleIPs")
            .as_array()
            .and_then(|ips| {
                ips.iter()
                    .filter_map(|i| i.as_str())
                    .find(|i| i.contains('.'))
                    .or_else(|| ips.first().and_then(|i| i.as_str()))
            })
            .map(str::to_string);
        let name = str_of(me.get("DNSName"))
            .map(|d| d.split('.').next().unwrap_or_default().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| str_of(me.get("HostName")));
        Some(TailPoll {
            backend: j
                .get("BackendState")
                .as_str()
                .unwrap_or("NoState")
                .to_string(),
            auth_url: str_of(j.get("AuthURL")),
            ip,
            name,
            tailnet: str_of(j.get("CurrentTailnet").get("Name")),
        })
    }
}

pub trait TailEngine: Send {
    /// The binaries are on the card.
    fn available(&self) -> bool;
    fn start(&mut self) -> Result<(), String>;
    fn alive(&mut self) -> bool;
    fn poll(&mut self) -> Option<TailPoll>;
    /// Asks to connect, which makes a login link if the device is not logged in yet.
    fn up(&mut self, hostname: &str);
    fn logout(&mut self);
    fn stop(&mut self);
}

pub struct TailPaths {
    pub daemon: PathBuf,
    pub cli: PathBuf,
    pub state: PathBuf,
    pub socket: PathBuf,
    pub log: PathBuf,
    /// Where the binaries are copied if the card will not run them in place.
    pub run: PathBuf,
}

impl TailPaths {
    pub fn on_card(card: &Path, run: &Path, scratch: &Path) -> TailPaths {
        let bin = card.join("System").join("bin");
        TailPaths {
            daemon: bin.join("tailscaled"),
            cli: bin.join("tailscale"),
            state: card.join("Config").join("tailscale"),
            socket: run.join("tailscaled.sock"),
            log: card.join("Logs").join("tailscale.log"),
            run: scratch.to_path_buf(),
        }
    }
}

pub struct Tailscale {
    paths: TailPaths,
    daemon: Option<Child>,
    cli: Option<PathBuf>,
    /// A running `tailscale up`, which waits for the login to finish.
    up: Option<(Child, Instant)>,
}

/// How long a `tailscale up` may wait for someone to log in before it is asked again.
const UP_LIMIT: Duration = Duration::from_secs(10 * 60);

impl Tailscale {
    pub fn new(paths: TailPaths) -> Tailscale {
        Tailscale {
            paths,
            daemon: None,
            cli: None,
            up: None,
        }
    }

    fn cli(&self) -> Option<Command> {
        let mut c = Command::new(self.cli.as_ref()?);
        c.arg(format!("--socket={}", self.paths.socket.display()))
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        Some(c)
    }

    fn reap_up(&mut self) {
        if let Some((child, since)) = &mut self.up {
            let done = !matches!(child.try_wait(), Ok(None));
            if done || since.elapsed() > UP_LIMIT {
                let _ = child.kill();
                let _ = child.wait();
                self.up = None;
            }
        }
    }
}

fn tun_available() -> bool {
    Path::new("/dev/net/tun").exists()
}

#[cfg(target_os = "linux")]
extern "C" {
    fn setpriority(which: i32, who: u32, prio: i32) -> i32;
}

impl TailEngine for Tailscale {
    fn available(&self) -> bool {
        self.paths.daemon.is_file() && self.paths.cli.is_file()
    }

    fn start(&mut self) -> Result<(), String> {
        if self.alive() {
            return Ok(());
        }
        let daemon = crate::syncthing::runnable(&self.paths.daemon, &self.paths.run)?;
        self.cli = Some(crate::syncthing::runnable(
            &self.paths.cli,
            &self.paths.run,
        )?);
        std::fs::create_dir_all(&self.paths.state).map_err(|e| e.to_string())?;
        if let Some(dir) = self.paths.socket.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Some(dir) = self.paths.log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let log = std::fs::File::create(&self.paths.log).map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        let mut c = Command::new(daemon);
        c.arg(format!("--statedir={}", self.paths.state.display()))
            .arg(format!("--socket={}", self.paths.socket.display()))
            .arg("--port=0")
            .arg("--no-logs-no-support");
        if !tun_available() {
            c.arg("--tun=userspace-networking");
        }
        let child = c
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(err)
            .spawn()
            .map_err(|e| format!("tailscaled: {e}"))?;
        #[cfg(target_os = "linux")]
        // SAFETY: PRIO_PROCESS (0) on a pid we just spawned; failure only leaves it at nice 0.
        unsafe {
            setpriority(0, child.id(), 10);
        }
        self.daemon = Some(child);
        Ok(())
    }

    fn alive(&mut self) -> bool {
        match self.daemon.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.daemon = None;
                false
            }
            None => false,
        }
    }

    fn poll(&mut self) -> Option<TailPoll> {
        self.reap_up();
        let out = self
            .cli()?
            .args(["status", "--json"])
            .stdout(Stdio::piped())
            .output()
            .ok()?;
        TailPoll::parse(&String::from_utf8_lossy(&out.stdout))
    }

    fn up(&mut self, hostname: &str) {
        self.reap_up();
        if self.up.is_some() {
            return;
        }
        let Some(mut c) = self.cli() else {
            return;
        };
        // DNS and the firewall stay BaseOS's: slot only wants the tailnet's addresses.
        let child = c
            .arg("up")
            .arg(format!("--hostname={hostname}"))
            .arg("--accept-dns=false")
            .arg("--netfilter-mode=off")
            .stdout(Stdio::null())
            .spawn();
        if let Ok(child) = child {
            self.up = Some((child, Instant::now()));
        }
    }

    fn logout(&mut self) {
        if let Some((mut child, _)) = self.up.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(mut c) = self.cli() {
            let _ = c.arg("logout").stdout(Stdio::null()).status();
        }
    }

    fn stop(&mut self) {
        if let Some((mut child, _)) = self.up.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(mut child) = self.daemon.take() {
            // SIGTERM lets tailscaled take its interface and routes down cleanly.
            #[cfg(unix)]
            {
                let _ = Command::new("kill")
                    .arg(child.id().to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let until = Instant::now() + Duration::from_secs(5);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Tailscale {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Stands in for Tailscale on the host build: it asks for a login, and once "visited"
/// (after a few polls) it is up.
#[derive(Default)]
pub struct SimTail {
    running: bool,
    logged_in: bool,
    asked: u32,
}

pub const SIM_LOGIN_URL: &str = "https://login.tailscale.com/a/sim1234abcd";

impl SimTail {
    pub fn new() -> SimTail {
        SimTail::default()
    }
}

impl TailEngine for SimTail {
    fn available(&self) -> bool {
        true
    }
    fn start(&mut self) -> Result<(), String> {
        self.running = true;
        Ok(())
    }
    fn alive(&mut self) -> bool {
        self.running
    }
    fn poll(&mut self) -> Option<TailPoll> {
        if !self.running {
            return None;
        }
        if self.asked > 0 && !self.logged_in {
            self.asked += 1;
            self.logged_in = self.asked > 3;
        }
        Some(match (self.logged_in, self.asked > 0) {
            (true, _) => TailPoll {
                backend: "Running".into(),
                auth_url: None,
                ip: Some("100.101.102.103".into()),
                name: Some("slot-rg35xxh".into()),
                tailnet: Some("example.com".into()),
            },
            (false, true) => TailPoll {
                backend: "NeedsLogin".into(),
                auth_url: Some(SIM_LOGIN_URL.into()),
                ..TailPoll::default()
            },
            (false, false) => TailPoll {
                backend: "NeedsLogin".into(),
                ..TailPoll::default()
            },
        })
    }
    fn up(&mut self, _hostname: &str) {
        if !self.logged_in && self.asked == 0 {
            self.asked = 1;
        }
    }
    fn logout(&mut self) {
        self.logged_in = false;
        self.asked = 0;
    }
    fn stop(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_json_gives_the_state_link_and_address() {
        let text = r#"{"Version":"1.102.3","BackendState":"Running","AuthURL":"",
            "Self":{"HostName":"slot","DNSName":"slot-rg35xxh.tail1234.ts.net.",
            "TailscaleIPs":["100.64.1.2","fd7a:115c:a1e0::1"]},
            "CurrentTailnet":{"Name":"me@example.com"}}"#;
        let p = TailPoll::parse(text).unwrap();
        assert_eq!(p.backend, "Running");
        assert_eq!(p.auth_url, None);
        assert_eq!(p.ip.as_deref(), Some("100.64.1.2"));
        assert_eq!(p.name.as_deref(), Some("slot-rg35xxh"));
        assert_eq!(p.tailnet.as_deref(), Some("me@example.com"));
    }

    #[test]
    fn a_device_waiting_for_login_carries_its_link() {
        let text = r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/abc",
            "Self":{"HostName":"slot","DNSName":"","TailscaleIPs":null},"CurrentTailnet":null}"#;
        let p = TailPoll::parse(text).unwrap();
        assert_eq!(p.backend, "NeedsLogin");
        assert_eq!(
            p.auth_url.as_deref(),
            Some("https://login.tailscale.com/a/abc")
        );
        assert_eq!(p.ip, None);
        assert_eq!(p.name.as_deref(), Some("slot"));
    }
}
