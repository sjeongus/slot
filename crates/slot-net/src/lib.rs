//! Wi-Fi and Syncthing for slot.
//!
//! [`Net`] runs a background thread that owns the supplicant and Syncthing. slot's UI sends it
//! commands and reads a [`NetStatus`] snapshot, so a slow radio never stalls a frame. Link play
//! borrows the radio through [`hold_for_link`] and [`release_after_link`].

pub mod config;
pub mod http;
pub mod json;
pub mod service;
pub mod syncthing;
pub mod wifi;
pub mod wpa;

#[cfg(test)]
mod testdir;

use std::path::{Path, PathBuf};

pub use config::{NetConfig, Network};
pub use service::{hold_for_link, release_after_link, Net, NetStatus, SyncState, WifiState};
pub use wpa::{valid_passphrase, ScanResult};

/// Runtime files: the supplicant config and control socket, DHCP pid, a runnable Syncthing.
pub fn run_dir() -> PathBuf {
    std::env::var_os("SLOT_RUN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"))
        .join("slotnet")
}

/// Starts the network thread for the card at `root`: the real radio and Syncthing on a
/// device build, simulated ones on the host.
pub fn start(root: &Path, country: Option<String>) -> Net {
    #[cfg(feature = "device")]
    {
        let run = run_dir();
        let wifi = wifi::ScriptWifi::new(root, &run);
        let cfg = NetConfig::load(root);
        let mut cfg_keys = (cfg.syncthing_api_key, cfg.syncthing_password);
        if cfg_keys.0.is_empty() || cfg_keys.1.is_empty() {
            // Net::start fills these in; make sure Syncthing gets the same ones.
            let mut c = NetConfig::load(root);
            if c.syncthing_api_key.is_empty() {
                c.syncthing_api_key = service::token(32);
            }
            if c.syncthing_password.is_empty() {
                c.syncthing_password = service::token(10);
            }
            let _ = c.save(root);
            cfg_keys = (c.syncthing_api_key, c.syncthing_password);
        }
        let sync = syncthing::Syncthing::new(
            syncthing::Paths::on_card(root, Path::new("/tmp/slotnet")),
            &cfg_keys.0,
            &cfg_keys.1,
        );
        Net::start(root, Box::new(wifi), Box::new(sync), country)
    }
    #[cfg(not(feature = "device"))]
    {
        Net::start(
            root,
            Box::new(wifi::SimWifi::new()),
            Box::new(syncthing::SimSync::new()),
            country,
        )
    }
}
