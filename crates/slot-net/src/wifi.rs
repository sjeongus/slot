//! Who actually touches the radio: `System/slotnet.sh` on a device, or a pretend radio on
//! the host build.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::wpa::{self, ScanResult, WpaStatus};

pub trait WifiDriver: Send {
    /// Starts the supplicant and DHCP client with this config, or reloads it if running.
    fn start(&mut self, conf: &str) -> Result<(), String>;
    fn stop(&mut self);
    fn alive(&mut self) -> bool;
    fn status(&mut self) -> Option<WpaStatus>;
    fn scan(&mut self) -> bool;
    fn scan_results(&mut self) -> Option<Vec<ScanResult>>;
    /// Joins `ssid` now, ahead of the others in the config.
    fn select(&mut self, ssid: &str) -> bool;
    /// Lets the supplicant roam between every saved network again.
    fn enable_all(&mut self);
    /// Asks DHCP for a fresh lease, after joining a different network.
    fn renew(&mut self);
}

/// Drives wpa_supplicant and udhcpc through `System/slotnet.sh` on the card.
pub struct ScriptWifi {
    script: PathBuf,
    run: PathBuf,
}

impl ScriptWifi {
    pub fn new(card: &Path, run: &Path) -> ScriptWifi {
        ScriptWifi {
            script: card.join("System").join("slotnet.sh"),
            run: run.to_path_buf(),
        }
    }

    fn sh(&self, args: &[&str]) -> Command {
        let mut c = Command::new("/bin/sh");
        c.arg(&self.script)
            .args(args)
            .env("SLOTNET_RUN", &self.run)
            .stdin(Stdio::null());
        c
    }

    fn ok(&self, args: &[&str]) -> bool {
        self.sh(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn out(&self, args: &[&str]) -> Option<String> {
        let o = self.sh(args).stderr(Stdio::null()).output().ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    fn cli(&self, args: &[&str]) -> Option<String> {
        let mut all = vec!["wifi", "cli"];
        all.extend_from_slice(args);
        self.out(&all)
    }

    fn conf_path(&self) -> PathBuf {
        self.run.join("wpa.conf")
    }
}

impl WifiDriver for ScriptWifi {
    fn start(&mut self, conf: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.run).map_err(|e| e.to_string())?;
        let path = self.conf_path();
        std::fs::write(&path, conf).map_err(|e| format!("write {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let p = path.to_string_lossy().into_owned();
        match self.ok(&["wifi", "start", &p]) {
            true => Ok(()),
            false => Err("Wi-Fi did not start".into()),
        }
    }

    fn stop(&mut self) {
        self.ok(&["wifi", "stop"]);
    }

    fn alive(&mut self) -> bool {
        self.ok(&["wifi", "alive"])
    }

    fn status(&mut self) -> Option<WpaStatus> {
        self.cli(&["status"]).map(|t| wpa::parse_status(&t))
    }

    fn scan(&mut self) -> bool {
        self.cli(&["scan"]).is_some_and(|t| t.trim() == "OK")
    }

    fn scan_results(&mut self) -> Option<Vec<ScanResult>> {
        self.cli(&["scan_results"]).map(|t| wpa::parse_scan(&t))
    }

    fn select(&mut self, ssid: &str) -> bool {
        let Some(list) = self.cli(&["list_networks"]) else {
            return false;
        };
        let Some(id) = wpa::network_id(&list, ssid) else {
            return false;
        };
        self.cli(&["select_network", &id.to_string()])
            .is_some_and(|t| t.trim() == "OK")
    }

    fn enable_all(&mut self) {
        self.cli(&["enable_network", "all"]);
    }

    fn renew(&mut self) {
        self.ok(&["dhcp", "renew"]);
    }
}

/// A radio that only exists in memory, for the host build and tests.
pub struct SimWifi {
    pub running: bool,
    conf: String,
    target: Option<String>,
    joined: Option<String>,
    pub nearby: Vec<(ScanResult, &'static str)>,
    scanned: bool,
    ticks: u32,
    pub renews: usize,
}

impl SimWifi {
    pub fn new() -> SimWifi {
        let net = |ssid: &str, dbm, secured, psk| {
            (
                ScanResult {
                    ssid: ssid.into(),
                    bssid: "02:00:00:00:00:01".into(),
                    freq: 2437,
                    signal_dbm: dbm,
                    secured,
                    sae_only: false,
                },
                psk,
            )
        };
        SimWifi {
            running: false,
            conf: String::new(),
            target: None,
            joined: None,
            nearby: vec![
                net("Home Wi-Fi", -48, true, "password1"),
                net("Coffee Shop", -71, false, ""),
                net("Neighbour 5G", -79, true, "unknowable"),
                net("Upstairs", -86, true, "unknowable"),
            ],
            scanned: false,
            ticks: 0,
            renews: 0,
        }
    }

    fn psk_in_conf(&self, ssid: &str) -> Option<String> {
        let hex = wpa::hex(ssid.as_bytes());
        let block = self
            .conf
            .split("network={")
            .find(|b| b.contains(&format!("ssid={hex}\n")))?;
        if block.contains("key_mgmt=NONE") {
            return Some(String::new());
        }
        let line = block.lines().find(|l| l.trim_start().starts_with("psk="))?;
        Some(line.trim_start()[4..].trim_matches('"').to_string())
    }

    fn try_join(&mut self, ssid: &str) {
        let Some((_, want)) = self.nearby.iter().find(|(r, _)| r.ssid == ssid) else {
            return;
        };
        if self.psk_in_conf(ssid).as_deref() == Some(want) {
            self.joined = Some(ssid.to_string());
        }
    }
}

impl Default for SimWifi {
    fn default() -> Self {
        Self::new()
    }
}

impl WifiDriver for SimWifi {
    fn start(&mut self, conf: &str) -> Result<(), String> {
        self.running = true;
        self.conf = conf.to_string();
        self.ticks = 0;
        if self
            .joined
            .as_deref()
            .is_some_and(|j| self.psk_in_conf(j).is_none())
        {
            self.joined = None;
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
        self.joined = None;
        self.target = None;
    }

    fn alive(&mut self) -> bool {
        self.running
    }

    fn status(&mut self) -> Option<WpaStatus> {
        if !self.running {
            return None;
        }
        self.ticks += 1;
        if self.joined.is_none() && self.ticks > 2 {
            match self.target.clone() {
                Some(t) => self.try_join(&t),
                None => {
                    let names: Vec<String> =
                        self.nearby.iter().map(|(r, _)| r.ssid.clone()).collect();
                    for n in names {
                        self.try_join(&n);
                        if self.joined.is_some() {
                            break;
                        }
                    }
                }
            }
        }
        Some(match &self.joined {
            Some(ssid) => WpaStatus {
                state: "COMPLETED".into(),
                ssid: Some(ssid.clone()),
                bssid: Some("02:00:00:00:00:01".into()),
                ip: Some("192.168.1.42".into()),
            },
            None if self.target.is_some() && self.ticks > 2 => WpaStatus {
                state: "DISCONNECTED".into(),
                ..WpaStatus::default()
            },
            None if self.target.is_some() => WpaStatus {
                state: "4WAY_HANDSHAKE".into(),
                ..WpaStatus::default()
            },
            None => WpaStatus {
                state: "SCANNING".into(),
                ..WpaStatus::default()
            },
        })
    }

    fn scan(&mut self) -> bool {
        self.scanned = self.running;
        self.running
    }

    fn scan_results(&mut self) -> Option<Vec<ScanResult>> {
        self.scanned
            .then(|| self.nearby.iter().map(|(r, _)| r.clone()).collect())
    }

    fn select(&mut self, ssid: &str) -> bool {
        self.target = Some(ssid.to_string());
        self.joined = None;
        self.ticks = 0;
        true
    }

    fn enable_all(&mut self) {
        self.target = None;
    }

    fn renew(&mut self) {
        self.renews += 1;
    }
}
