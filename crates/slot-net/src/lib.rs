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
pub mod tailscale;
pub mod time;
pub mod wifi;
pub mod wpa;

#[cfg(test)]
mod testdir;

use std::path::{Path, PathBuf};

pub use config::{NetConfig, Network};
pub use service::{
    hold_for_link, online, release_after_link, Backends, Net, NetStatus, SyncState, TailState,
    WifiState,
};
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
        let tail = tailscale::Tailscale::new(tailscale::TailPaths::on_card(
            root,
            &run,
            Path::new("/tmp/slotnet"),
        ));
        Net::start(
            root,
            Backends {
                wifi: Box::new(wifi),
                sync: Box::new(sync),
                clock: Box::new(time::BaseosClock::new()),
                tail: Box::new(tail),
                probe: Box::new(service::tcp_probe),
                hostname: hostname(),
            },
            country,
        )
    }
    #[cfg(not(feature = "device"))]
    {
        Net::start(root, sim_backends(), country)
    }
}

/// Stand-ins for everything, for the host build and tests.
pub fn sim_backends() -> Backends {
    Backends {
        wifi: Box::new(wifi::SimWifi::new()),
        sync: Box::new(syncthing::SimSync::new()),
        clock: Box::new(time::SimClock::default()),
        tail: Box::new(tailscale::SimTail::new()),
        probe: Box::new(|| true),
        hostname: "slot".into(),
    }
}

/// "slot-rg35xxh": the model BaseOS names, as Tailscale shows this device.
pub fn hostname() -> String {
    let model = std::env::var("BASEOS_MODEL").ok().or_else(|| {
        std::fs::read_to_string("/etc/baseos-release")
            .ok()?
            .lines()
            .find_map(|l| l.trim().strip_prefix("BASEOS_MODEL=").map(str::to_string))
    });
    let model = model.unwrap_or_default().replace('"', "");
    let model: String = model
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    match model.is_empty() {
        true => "slot".into(),
        false => format!("slot-{model}"),
    }
}
