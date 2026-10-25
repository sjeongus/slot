//! `Config/net.txt` on the card: what the person turned on, and the networks they saved.
//!
//! The file is plain text so it can be edited from a computer:
//!
//! ```text
//! wifi=on
//! syncthing=on
//! sync_in_game=off
//!
//! [network]
//! ssid=Home
//! psk=correct horse battery staple
//! ```
//!
//! Everything after the first `=` is the value, kept as is apart from a trailing `\r`, so
//! passphrases may hold `=`, `#` and spaces. Unknown keys are kept and written back.

use std::path::{Path, PathBuf};

pub const FILE: &str = "net.txt";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// Empty for an open network.
    pub psk: String,
    /// Not shown in scans, so the supplicant has to probe for it by name.
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConfig {
    pub wifi: bool,
    pub syncthing: bool,
    /// Let Syncthing touch Saves and States while a game is running.
    pub sync_in_game: bool,
    /// Most recently joined first; the supplicant prefers them in this order.
    pub networks: Vec<Network>,
    /// Chosen once, so the web UI password and the API key survive restarts.
    pub syncthing_api_key: String,
    pub syncthing_password: String,
    extra: Vec<(String, String)>,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            wifi: true,
            syncthing: false,
            sync_in_game: false,
            networks: Vec::new(),
            syncthing_api_key: String::new(),
            syncthing_password: String::new(),
            extra: Vec::new(),
        }
    }
}

fn flag(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

impl NetConfig {
    pub fn path(root: &Path) -> PathBuf {
        root.join("Config").join(FILE)
    }

    pub fn load(root: &Path) -> NetConfig {
        std::fs::read_to_string(Self::path(root))
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_file_name(format!(".{FILE}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, self.render())?;
        if let Ok(f) = std::fs::File::open(&tmp) {
            let _ = f.sync_all();
        }
        std::fs::rename(&tmp, &path)
    }

    pub fn parse(text: &str) -> NetConfig {
        let mut c = NetConfig::default();
        let mut current: Option<Network> = None;
        for raw in text.lines() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.eq_ignore_ascii_case("[network]") {
                if let Some(n) = current.take().filter(|n| !n.ssid.is_empty()) {
                    c.networks.push(n);
                }
                current = Some(Network {
                    ssid: String::new(),
                    psk: String::new(),
                    hidden: false,
                });
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            if let Some(n) = current.as_mut() {
                match key.as_str() {
                    "ssid" => n.ssid = v.to_string(),
                    "psk" | "password" => n.psk = v.to_string(),
                    "hidden" => n.hidden = flag(v).unwrap_or(false),
                    _ => {}
                }
                continue;
            }
            match key.as_str() {
                "wifi" => c.wifi = flag(v).unwrap_or(c.wifi),
                "syncthing" => c.syncthing = flag(v).unwrap_or(c.syncthing),
                "sync_in_game" => c.sync_in_game = flag(v).unwrap_or(c.sync_in_game),
                "syncthing_api_key" => c.syncthing_api_key = v.trim().to_string(),
                "syncthing_password" => c.syncthing_password = v.trim().to_string(),
                _ => c.extra.push((k.trim().to_string(), v.to_string())),
            }
        }
        if let Some(n) = current.filter(|n| !n.ssid.is_empty()) {
            c.networks.push(n);
        }
        c
    }

    pub fn render(&self) -> String {
        let mut s = String::from(
            "# slot network settings. Safe to edit while slot is off.\n\
             # Passwords are stored as plain text on this card.\n",
        );
        s += &format!("wifi={}\n", on_off(self.wifi));
        s += &format!("syncthing={}\n", on_off(self.syncthing));
        s += &format!("sync_in_game={}\n", on_off(self.sync_in_game));
        if !self.syncthing_api_key.is_empty() {
            s += &format!("syncthing_api_key={}\n", self.syncthing_api_key);
        }
        if !self.syncthing_password.is_empty() {
            s += &format!("syncthing_password={}\n", self.syncthing_password);
        }
        for (k, v) in &self.extra {
            s += &format!("{k}={v}\n");
        }
        for n in &self.networks {
            s += "\n[network]\n";
            s += &format!("ssid={}\n", n.ssid);
            if !n.psk.is_empty() {
                s += &format!("psk={}\n", n.psk);
            }
            if n.hidden {
                s += "hidden=on\n";
            }
        }
        s
    }

    pub fn network(&self, ssid: &str) -> Option<&Network> {
        self.networks.iter().find(|n| n.ssid == ssid)
    }

    /// Saves `net`, replacing any network of the same name, and puts it first.
    pub fn remember(&mut self, net: Network) {
        self.networks.retain(|n| n.ssid != net.ssid);
        self.networks.insert(0, net);
    }

    /// Moves an already saved network to the front without touching its password.
    pub fn prefer(&mut self, ssid: &str) {
        if let Some(at) = self.networks.iter().position(|n| n.ssid == ssid) {
            let n = self.networks.remove(at);
            self.networks.insert(0, n);
        }
    }

    pub fn forget(&mut self, ssid: &str) -> bool {
        let before = self.networks.len();
        self.networks.retain(|n| n.ssid != ssid);
        before != self.networks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_awkward_passwords_and_names() {
        let mut c = NetConfig::default();
        c.remember(Network {
            ssid: "Café = home #2".into(),
            psk: "  pass=word # not a comment ".into(),
            hidden: true,
        });
        c.remember(Network {
            ssid: "Open".into(),
            psk: String::new(),
            hidden: false,
        });
        let back = NetConfig::parse(&c.render());
        assert_eq!(back.networks, c.networks);
        assert_eq!(back.networks[0].ssid, "Open");
    }

    #[test]
    fn missing_file_means_wifi_on_and_syncthing_off() {
        let d = crate::testdir::TestDir::new("net");
        let c = NetConfig::load(d.path());
        assert!(c.wifi);
        assert!(!c.syncthing);
        assert!(c.networks.is_empty());
    }

    #[test]
    fn saves_into_config_and_reads_back() {
        let d = crate::testdir::TestDir::new("net");
        let c = NetConfig {
            syncthing: true,
            syncthing_api_key: "abc".into(),
            ..NetConfig::default()
        };
        c.save(d.path()).unwrap();
        let back = NetConfig::load(d.path());
        assert!(back.syncthing);
        assert_eq!(back.syncthing_api_key, "abc");
    }

    #[test]
    fn crlf_from_a_windows_editor_is_tolerated() {
        let c = NetConfig::parse("wifi=off\r\n[network]\r\nssid=A\r\npsk=b c\r\n");
        assert!(!c.wifi);
        assert_eq!(c.networks[0].ssid, "A");
        assert_eq!(c.networks[0].psk, "b c");
    }

    #[test]
    fn remembering_again_moves_to_front_and_forgetting_removes() {
        let mut c = NetConfig::default();
        for s in ["a", "b", "c"] {
            c.remember(Network {
                ssid: s.into(),
                psk: "12345678".into(),
                hidden: false,
            });
        }
        c.prefer("a");
        assert_eq!(c.networks[0].ssid, "a");
        assert!(c.forget("b"));
        assert!(!c.forget("b"));
        assert_eq!(c.networks.len(), 2);
    }

    #[test]
    fn unknown_keys_survive_a_rewrite() {
        let c = NetConfig::parse("future_thing=42\n");
        assert!(c.render().contains("future_thing=42"));
    }
}
