//! Running Syncthing next to slot and steering it through its REST API.
//!
//! Its keys and config live on the card in `Config/syncthing`, so the handheld keeps its
//! device ID when the card moves or BaseOS is reflashed. The database is only an index and
//! is rebuilt if lost, so it goes on BaseOS's ext4 `/data` partition when there is one,
//! where SQLite is happier than on FAT32.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::http;
use crate::json::quote;

pub const GUI_PORT: u16 = 8384;
pub const GUI_USER: &str = "slot";

/// The folders slot offers to Syncthing: (folder ID, label, directory under the card).
pub const FOLDERS: [(&str, &str, &str); 2] = [
    ("slot-saves", "slot Saves", "Saves"),
    ("slot-states", "slot States", "States"),
];

/// slot writes through `.<name>.<pid>.<seq>.tmp` and renames; Syncthing should never send those.
pub const STIGNORE: &str =
    "// Written by slot. Temporary files from atomic writes.\n(?d).*.tmp\n(?d)*.tmp\n";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncPoll {
    /// Other devices currently connected.
    pub devices: usize,
    /// Any of slot's folders scanning or pulling.
    pub busy: bool,
    pub error: Option<String>,
}

pub trait SyncEngine: Send {
    /// The binary is on the card.
    fn available(&self) -> bool;
    fn start(&mut self) -> Result<(), String>;
    /// The process is still alive.
    fn alive(&mut self) -> bool;
    /// The REST API answers.
    fn ready(&mut self) -> bool;
    fn device_id(&mut self) -> Option<String>;
    fn ensure_folders(&mut self) -> Result<(), String>;
    fn set_paused(&mut self, paused: bool) -> Result<(), String>;
    fn scan(&mut self);
    fn poll(&mut self) -> Option<SyncPoll>;
    fn stop(&mut self);
}

pub struct Paths {
    pub binary: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub card: PathBuf,
    pub log: PathBuf,
    /// Where the binary is copied if the card will not let it run in place. BaseOS's /run is
    /// a 32 MB tmpfs, too small for Syncthing, so this is under /tmp (256 MB).
    pub run: PathBuf,
}

impl Paths {
    pub fn on_card(card: &Path, scratch: &Path) -> Paths {
        let config = card.join("Config").join("syncthing");
        let data = data_dir().unwrap_or_else(|| config.clone());
        Paths {
            binary: card.join("System").join("bin").join("syncthing"),
            config,
            data,
            card: card.to_path_buf(),
            log: card.join("Logs").join("syncthing.log"),
            run: scratch.to_path_buf(),
        }
    }
}

/// BaseOS mounts its ext4 p6 at /data, or a tmpfs there if that fails. The index only goes
/// there when it is the real partition, so it survives a restart.
fn data_dir() -> Option<PathBuf> {
    let base = Path::new("/data");
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    if !mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some("/data") && l.contains(" ext4 "))
    {
        return None;
    }
    let dir = base.join("slot").join("syncthing");
    std::fs::create_dir_all(&dir).ok()?;
    let probe = dir.join(".probe");
    std::fs::write(&probe, b"").ok()?;
    let _ = std::fs::remove_file(probe);
    Some(dir)
}

pub struct Syncthing {
    paths: Paths,
    api_key: String,
    password: String,
    child: Option<Child>,
    addr: SocketAddr,
    device_id: Option<String>,
    folders_ready: bool,
}

impl Syncthing {
    pub fn new(paths: Paths, api_key: &str, password: &str) -> Syncthing {
        Syncthing {
            paths,
            api_key: api_key.to_string(),
            password: password.to_string(),
            child: None,
            addr: SocketAddr::from(([127, 0, 0, 1], GUI_PORT)),
            device_id: None,
            folders_ready: false,
        }
    }

    fn rest(&self, method: &str, path: &str, body: Option<&str>) -> Option<http::Response> {
        http::request(
            self.addr,
            method,
            path,
            &self.api_key,
            body,
            Duration::from_secs(3),
        )
        .ok()
    }

    fn command(&self, binary: &Path) -> Command {
        let mut c = Command::new(binary);
        c.env("HOME", &self.paths.card)
            .env("STNODEFAULTFOLDER", "1")
            .env("STNOUPGRADE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    }

    /// Writes keys and a config the first time, with the web UI behind a password.
    fn generate(&self, binary: &Path) -> Result<(), String> {
        if self.paths.config.join("config.xml").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.paths.config).map_err(|e| e.to_string())?;
        let mut child = self
            .command(binary)
            .arg("generate")
            .arg(format!("--config={}", self.paths.config.display()))
            .arg(format!("--data={}", self.paths.data.display()))
            .arg(format!("--gui-user={GUI_USER}"))
            .arg("--gui-password=-")
            .arg("--no-port-probing")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("syncthing generate: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(self.password.as_bytes());
        }
        match wait_for(&mut child, Duration::from_secs(60)) {
            Some(s) if s.success() => Ok(()),
            Some(s) => Err(format!("syncthing generate exited {s}")),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                Err("syncthing generate took too long".into())
            }
        }
    }

    /// The card may be mounted without exec permission; a copy in tmpfs always runs.
    fn runnable(&self) -> Result<PathBuf, String> {
        let bin = &self.paths.binary;
        if probe(bin) {
            return Ok(bin.clone());
        }
        let copy = self.paths.run.join("syncthing");
        let fresh = match (std::fs::metadata(bin), std::fs::metadata(&copy)) {
            (Ok(a), Ok(b)) => a.len() == b.len(),
            _ => false,
        };
        if !fresh {
            std::fs::create_dir_all(&self.paths.run).map_err(|e| e.to_string())?;
            std::fs::copy(bin, &copy).map_err(|e| format!("copy syncthing: {e}"))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755));
        }
        Ok(copy)
    }
}

fn probe(bin: &Path) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait_for(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => return None,
        }
    }
}

pub fn folder_json(id: &str, label: &str, path: &Path, versioned: bool, paused: bool) -> String {
    let versioning = if versioned {
        r#","versioning":{"type":"simple","params":{"keep":"5","cleanoutDays":"60"}}"#
    } else {
        ""
    };
    format!(
        r#"{{"id":{},"label":{},"path":{},"type":"sendreceive","fsWatcherEnabled":true,"fsWatcherDelayS":5,"rescanIntervalS":3600,"paused":{paused}{versioning}}}"#,
        quote(id),
        quote(label),
        quote(&path.to_string_lossy()),
    )
}

#[cfg(target_os = "linux")]
extern "C" {
    fn setpriority(which: i32, who: u32, prio: i32) -> i32;
}

impl SyncEngine for Syncthing {
    fn available(&self) -> bool {
        self.paths.binary.is_file()
    }

    fn start(&mut self) -> Result<(), String> {
        if self.alive() {
            return Ok(());
        }
        let bin = self.runnable()?;
        self.generate(&bin)?;
        if let Some(dir) = self.paths.log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let child = self
            .command(&bin)
            .arg("serve")
            .arg(format!("--config={}", self.paths.config.display()))
            .arg(format!("--data={}", self.paths.data.display()))
            .arg(format!("--gui-address=http://0.0.0.0:{GUI_PORT}"))
            .arg(format!("--gui-apikey={}", self.api_key))
            .arg("--no-browser")
            .arg("--no-restart")
            .arg("--no-upgrade")
            .arg(format!("--log-file={}", self.paths.log.display()))
            .arg("--log-max-size=1048576")
            .arg("--log-max-old-files=1")
            .spawn()
            .map_err(|e| format!("syncthing: {e}"))?;
        #[cfg(target_os = "linux")]
        // SAFETY: PRIO_PROCESS (0) on a pid we just spawned; failure only leaves it at nice 0.
        // Games come first; Syncthing gets what is left over.
        unsafe {
            setpriority(0, child.id(), 10);
        }
        self.child = Some(child);
        self.folders_ready = false;
        Ok(())
    }

    fn alive(&mut self) -> bool {
        match self.child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                false
            }
            None => false,
        }
    }

    fn ready(&mut self) -> bool {
        self.rest("GET", "/rest/system/ping", None)
            .is_some_and(|r| r.ok())
    }

    fn device_id(&mut self) -> Option<String> {
        if self.device_id.is_none() {
            self.device_id = self
                .rest("GET", "/rest/system/status", None)
                .and_then(|r| r.json())
                .and_then(|j| j.get("myID").as_str().map(str::to_string));
        }
        self.device_id.clone()
    }

    fn ensure_folders(&mut self) -> Result<(), String> {
        if self.folders_ready {
            return Ok(());
        }
        let have = self
            .rest("GET", "/rest/config/folders", None)
            .and_then(|r| r.json())
            .ok_or("Syncthing did not list its folders")?;
        let ids: Vec<&str> = have
            .as_array()
            .map(|a| a.iter().filter_map(|f| f.get("id").as_str()).collect())
            .unwrap_or_default();
        for (id, label, dir) in FOLDERS {
            let path = self.paths.card.join(dir);
            let _ = std::fs::create_dir_all(&path);
            let ignore = path.join(".stignore");
            if !ignore.exists() {
                let _ = std::fs::write(&ignore, STIGNORE);
            }
            if ids.contains(&id) {
                continue;
            }
            let body = folder_json(id, label, &path, id == "slot-saves", false);
            match self.rest("POST", "/rest/config/folders", Some(&body)) {
                Some(r) if r.ok() => {}
                Some(r) => return Err(format!("adding {label}: {} {}", r.status, r.body.trim())),
                None => return Err(format!("adding {label}: no answer")),
            }
        }
        self.folders_ready = true;
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), String> {
        let body = format!("{{\"paused\":{paused}}}");
        for (id, ..) in FOLDERS {
            match self.rest("PATCH", &format!("/rest/config/folders/{id}"), Some(&body)) {
                Some(r) if r.ok() => {}
                Some(r) => return Err(format!("{id}: {} {}", r.status, r.body.trim())),
                None => return Err(format!("{id}: no answer")),
            }
        }
        Ok(())
    }

    fn scan(&mut self) {
        for (id, ..) in FOLDERS {
            let _ = self.rest("POST", &format!("/rest/db/scan?folder={id}"), None);
        }
    }

    fn poll(&mut self) -> Option<SyncPoll> {
        let conns = self.rest("GET", "/rest/system/connections", None)?.json()?;
        let devices = conns
            .get("connections")
            .as_object()
            .map(|m| {
                m.values()
                    .filter(|c| c.get("connected").as_bool() == Some(true))
                    .count()
            })
            .unwrap_or(0);
        let mut poll = SyncPoll {
            devices,
            ..SyncPoll::default()
        };
        for (id, ..) in FOLDERS {
            let Some(s) = self
                .rest("GET", &format!("/rest/db/status?folder={id}"), None)
                .and_then(|r| r.json())
            else {
                continue;
            };
            match s.get("state").as_str().unwrap_or("") {
                "scanning" | "syncing" | "sync-preparing" | "scan-waiting" | "sync-waiting" => {
                    poll.busy = true
                }
                "error" => {
                    poll.error = Some(s.get("error").as_str().unwrap_or("error").to_string())
                }
                _ => {}
            }
        }
        Some(poll)
    }

    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.rest("POST", "/rest/system/shutdown", None);
        if wait_for(&mut child, Duration::from_secs(5)).is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.folders_ready = false;
    }
}

impl Drop for Syncthing {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Stands in for Syncthing on the host build.
pub struct SimSync {
    started: Option<Instant>,
    pub paused: bool,
    pub scans: usize,
}

impl SimSync {
    pub fn new() -> SimSync {
        SimSync {
            started: None,
            paused: false,
            scans: 0,
        }
    }
}

impl Default for SimSync {
    fn default() -> Self {
        Self::new()
    }
}

pub const SIM_DEVICE_ID: &str = "MFZWI3D-BONSGYC-YLTMRWG-C43ENR5-QXGZDMM-FZWI3DP-BONSGYY-LTMRWAD";

impl SyncEngine for SimSync {
    fn available(&self) -> bool {
        true
    }

    fn start(&mut self) -> Result<(), String> {
        self.started.get_or_insert_with(Instant::now);
        Ok(())
    }

    fn alive(&mut self) -> bool {
        self.started.is_some()
    }

    fn ready(&mut self) -> bool {
        self.started
            .is_some_and(|t| t.elapsed() > Duration::from_millis(1500))
    }

    fn device_id(&mut self) -> Option<String> {
        self.ready().then(|| SIM_DEVICE_ID.to_string())
    }

    fn ensure_folders(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), String> {
        self.paused = paused;
        Ok(())
    }

    fn scan(&mut self) {
        self.scans += 1;
    }

    fn poll(&mut self) -> Option<SyncPoll> {
        self.ready().then_some(SyncPoll {
            devices: 1,
            busy: false,
            error: None,
        })
    }

    fn stop(&mut self) {
        self.started = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Json;

    #[test]
    fn saves_are_versioned_and_states_are_not() {
        let s = Json::parse(&folder_json(
            "slot-saves",
            "x",
            Path::new("/c/Saves"),
            true,
            true,
        ))
        .unwrap();
        assert_eq!(s.get("path").as_str(), Some("/c/Saves"));
        assert_eq!(s.get("paused").as_bool(), Some(true));
        assert_eq!(s.get("versioning").get("type").as_str(), Some("simple"));
        let t = Json::parse(&folder_json(
            "slot-states",
            "x",
            Path::new("/c"),
            false,
            false,
        ))
        .unwrap();
        assert_eq!(t.get("versioning"), &Json::Null);
    }

    #[test]
    fn the_card_keeps_keys_and_the_binary() {
        let p = Paths::on_card(Path::new("/mnt/sdcard"), Path::new("/run/slotnet"));
        assert_eq!(p.binary, Path::new("/mnt/sdcard/System/bin/syncthing"));
        assert_eq!(p.config, Path::new("/mnt/sdcard/Config/syncthing"));
        assert_eq!(p.log, Path::new("/mnt/sdcard/Logs/syncthing.log"));
    }

    /// Runs the real thing when `SLOT_TEST_SYNCTHING` names a syncthing binary for this host.
    #[test]
    fn drives_a_real_syncthing_when_one_is_given() {
        let Some(bin) = std::env::var_os("SLOT_TEST_SYNCTHING") else {
            return;
        };
        let card = crate::testdir::TestDir::new("st-card");
        let run = crate::testdir::TestDir::new("st-run");
        let paths = Paths {
            binary: PathBuf::from(bin),
            config: card.path().join("Config/syncthing"),
            data: card.path().join("Config/syncthing"),
            card: card.path().to_path_buf(),
            log: card.path().join("Logs/syncthing.log"),
            run: run.path().to_path_buf(),
        };
        let mut st = Syncthing::new(paths, "slot-test-key", "slot-test-password");
        st.addr = SocketAddr::from(([127, 0, 0, 1], GUI_PORT));
        st.start().unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        while !st.ready() {
            assert!(Instant::now() < until, "syncthing never answered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let id = st.device_id().unwrap();
        assert_eq!(id.len(), 63);
        st.ensure_folders().unwrap();
        assert!(card.path().join("Saves/.stignore").exists());
        st.set_paused(true).unwrap();
        st.set_paused(false).unwrap();
        st.scan();
        let poll = st.poll().unwrap();
        assert_eq!(poll.devices, 0);
        st.stop();
        assert!(!st.alive());
        let config =
            std::fs::read_to_string(card.path().join("Config/syncthing/config.xml")).unwrap();
        assert!(config.contains("slot-saves"));
        assert!(config.contains("<user>slot</user>"));
    }
}
