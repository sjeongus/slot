//! The network thread. slot's UI never waits on the radio: it sends commands here and reads
//! a snapshot of [`NetStatus`] each frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::config::{NetConfig, Network};
use crate::syncthing::{SyncEngine, GUI_PORT};
use crate::tailscale::{TailEngine, TailPoll};
use crate::time::Clock;
use crate::wifi::WifiDriver;
use crate::wpa::{self, ScanResult};

/// How long a new network gets to associate and hand out an address before slot gives up.
const JOIN_LIMIT: Duration = Duration::from_secs(25);
/// How long after `scan` the supplicant has results worth reading.
const SCAN_SETTLE: Duration = Duration::from_millis(2500);
const SCAN_LIMIT: Duration = Duration::from_secs(10);
/// Poll periods: quick while a network screen is open or something is changing.
const FAST: Duration = Duration::from_millis(700);
const SLOW: Duration = Duration::from_secs(8);
const SYNC_POLL_FAST: Duration = Duration::from_secs(3);
const SYNC_POLL_SLOW: Duration = Duration::from_secs(30);
const SYNC_START_LIMIT: Duration = Duration::from_secs(90);
/// How often the time zone is looked up again while online, and how soon after a failure.
const ZONE_EVERY: Duration = Duration::from_secs(24 * 3600);
const ZONE_RETRY: Duration = Duration::from_secs(10 * 60);
const SYNCED_POLL: Duration = Duration::from_secs(30);
/// Link play waits at most this long for slot's Wi-Fi to step aside.
const HOLD_LIMIT: Duration = Duration::from_secs(10);
/// How often the internet is checked while connected, and sooner after a miss.
const PROBE_EVERY: Duration = Duration::from_secs(60);
const PROBE_RETRY: Duration = Duration::from_secs(15);
const TAIL_POLL_FAST: Duration = Duration::from_secs(2);
const TAIL_POLL_SLOW: Duration = Duration::from_secs(30);

static ONLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wi-Fi is connected and the internet answered the last check. Read by anything that
/// would rather not wait on a request that cannot work, such as RetroAchievements.
pub fn online() -> bool {
    ONLINE.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WifiState {
    Off,
    /// Lent to link play, which needs the radio to itself.
    Linking,
    /// On, with nothing saved to join.
    NoNetworks,
    Searching,
    Joining {
        ssid: String,
    },
    Connected {
        ssid: String,
        ip: Option<String>,
    },
    Failed {
        ssid: String,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    Off,
    /// `System/bin/syncthing` is not on the card.
    Missing,
    WaitingForWifi,
    Starting,
    /// Paused while a game is running.
    Paused,
    Running {
        devices: usize,
        busy: bool,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TailState {
    Off,
    /// The Tailscale binaries are not on the card.
    Missing,
    WaitingForWifi,
    Starting,
    /// Not logged in. The link, once Tailscale has made one, logs this device in.
    NeedsLogin {
        url: Option<String>,
    },
    /// Logged in; an admin has to approve the device on the tailnet.
    NeedsApproval,
    Running {
        ip: Option<String>,
        name: Option<String>,
        tailnet: Option<String>,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetStatus {
    pub wifi_on: bool,
    pub wifi: WifiState,
    pub scan: Vec<ScanResult>,
    pub scanning: bool,
    pub saved: Vec<String>,
    pub syncthing_on: bool,
    pub sync_in_game: bool,
    pub sync: SyncState,
    pub device_id: Option<String>,
    pub web_password: String,
    pub network_time: bool,
    /// NTP has set the clock since boot.
    pub time_synced: bool,
    pub auto_timezone: bool,
    /// The zone in use, found or remembered; None until one is known.
    pub timezone: Option<String>,
    pub retroarch_saves: bool,
    /// Connected, and the internet answered the last check.
    pub online: bool,
    pub tailscale_on: bool,
    pub tail: TailState,
    /// Bumped on every change so screens know to redraw their text.
    pub generation: u64,
}

impl NetStatus {
    pub fn ip(&self) -> Option<&str> {
        match &self.wifi {
            WifiState::Connected { ip, .. } => ip.as_deref(),
            _ => None,
        }
    }

    pub fn connected_ssid(&self) -> Option<&str> {
        match &self.wifi {
            WifiState::Connected { ssid, .. } => Some(ssid),
            _ => None,
        }
    }

    pub fn web_url(&self) -> Option<String> {
        self.ip().map(|ip| format!("http://{ip}:{GUI_PORT}"))
    }
}

#[derive(Debug)]
enum Cmd {
    SetWifi(bool),
    Scan,
    Join(Network),
    JoinSaved(String),
    Forget(String),
    SetSyncthing(bool),
    SetSyncInGame(bool),
    SetNetworkTime(bool),
    SetAutoTimezone(bool),
    SetRetroArchSaves(bool),
    Playing(bool),
    Watching(bool),
    Hold(Sender<()>),
    Release,
    Shutdown(Sender<()>),
    SetTailscale(bool),
    TailLogout,
}

/// What the network thread drives. On a device build these are the real radio, Syncthing,
/// clock and Tailscale; on the host, stand-ins.
pub struct Backends {
    pub wifi: Box<dyn WifiDriver>,
    pub sync: Box<dyn SyncEngine>,
    pub clock: Box<dyn Clock>,
    pub tail: Box<dyn TailEngine>,
    /// Can the internet be reached? Called only while Wi-Fi is connected.
    pub probe: Box<dyn Fn() -> bool + Send>,
    /// The name Tailscale gives this device.
    pub hostname: String,
}

/// Tries to open a TCP connection to well-known public DNS servers.
pub fn tcp_probe() -> bool {
    use std::net::{SocketAddr, TcpStream};
    [
        ([1, 1, 1, 1], 443),
        ([8, 8, 8, 8], 443),
        ([9, 9, 9, 9], 443),
    ]
    .into_iter()
    .any(|(ip, port)| {
        TcpStream::connect_timeout(&SocketAddr::from((ip, port)), Duration::from_secs(3)).is_ok()
    })
}

/// The handle slot keeps. Cheap to call from the UI thread: every method only sends.
#[derive(Clone)]
pub struct Net {
    tx: Sender<Cmd>,
    status: Arc<Mutex<NetStatus>>,
}

fn global() -> &'static Mutex<Option<Net>> {
    static G: OnceLock<Mutex<Option<Net>>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(None))
}

/// Link play calls this before it takes the radio. slot's own Wi-Fi and Syncthing stop, and
/// it returns once they have, or after a few seconds regardless.
pub fn hold_for_link() {
    let net = global().lock().ok().and_then(|g| g.clone());
    if let Some(net) = net {
        net.hold();
    }
}

/// Link play calls this once the radio is free again.
pub fn release_after_link() {
    if let Some(net) = global().lock().ok().and_then(|g| g.clone()) {
        net.release();
    }
}

impl Net {
    pub fn start(root: &Path, backends: Backends, country: Option<String>) -> Net {
        let Backends {
            wifi,
            sync,
            clock,
            tail,
            probe,
            hostname,
        } = backends;
        let mut config = NetConfig::load(root);
        let mut changed = false;
        if config.syncthing_api_key.is_empty() {
            config.syncthing_api_key = token(32);
            changed = true;
        }
        if config.syncthing_password.is_empty() {
            config.syncthing_password = token(10);
            changed = true;
        }
        if changed {
            let _ = config.save(root);
        }
        let status = Arc::new(Mutex::new(NetStatus {
            wifi_on: config.wifi,
            wifi: WifiState::Off,
            scan: Vec::new(),
            scanning: false,
            saved: config.networks.iter().map(|n| n.ssid.clone()).collect(),
            syncthing_on: config.syncthing,
            sync_in_game: config.sync_in_game,
            sync: SyncState::Off,
            device_id: None,
            web_password: config.syncthing_password.clone(),
            network_time: config.network_time,
            time_synced: false,
            auto_timezone: config.auto_timezone,
            timezone: Some(config.timezone.clone()).filter(|z| !z.is_empty()),
            retroarch_saves: config.retroarch_saves,
            online: false,
            tailscale_on: config.tailscale,
            tail: TailState::Off,
            generation: 1,
        }));
        let (tx, rx) = channel();
        let worker = Worker {
            root: root.to_path_buf(),
            config,
            wifi,
            sync,
            clock,
            status: status.clone(),
            country,
            running: false,
            held: false,
            playing: false,
            watching: false,
            pending: None,
            last_ssid: None,
            scan_asked: None,
            scan_wanted: None,
            sync_started: None,
            sync_poll_at: None,
            sync_paused: None,
            sync_ready: false,
            sync_failed: None,
            zone_checked: None,
            synced_polled: None,
            tail,
            tail_started: false,
            tail_poll_at: None,
            tail_failed: None,
            hostname,
            probe,
            probe_at: None,
            probe_ok: false,
        };
        let _ = std::thread::Builder::new()
            .name("slot-net".into())
            .spawn(move || worker.run(rx));
        let net = Net { tx, status };
        if let Ok(mut g) = global().lock() {
            *g = Some(net.clone());
        }
        net
    }

    pub fn status(&self) -> NetStatus {
        lock(&self.status).clone()
    }

    pub fn generation(&self) -> u64 {
        lock(&self.status).generation
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn set_wifi(&self, on: bool) {
        self.send(Cmd::SetWifi(on))
    }

    pub fn scan(&self) {
        self.send(Cmd::Scan)
    }

    /// Joins a network with this password, saving it once the join works.
    pub fn join(&self, ssid: &str, psk: &str, hidden: bool) {
        self.send(Cmd::Join(Network {
            ssid: ssid.to_string(),
            psk: psk.to_string(),
            hidden,
        }))
    }

    pub fn join_saved(&self, ssid: &str) {
        self.send(Cmd::JoinSaved(ssid.to_string()))
    }

    pub fn forget(&self, ssid: &str) {
        self.send(Cmd::Forget(ssid.to_string()))
    }

    pub fn set_syncthing(&self, on: bool) {
        self.send(Cmd::SetSyncthing(on))
    }

    pub fn set_sync_in_game(&self, on: bool) {
        self.send(Cmd::SetSyncInGame(on))
    }

    pub fn set_network_time(&self, on: bool) {
        self.send(Cmd::SetNetworkTime(on))
    }

    pub fn set_auto_timezone(&self, on: bool) {
        self.send(Cmd::SetAutoTimezone(on))
    }

    /// Records the save layout choice; slot switches its own layout at the same time.
    pub fn set_retroarch_saves(&self, on: bool) {
        self.send(Cmd::SetRetroArchSaves(on))
    }

    /// A game is running (true) or the cart came out (false).
    pub fn playing(&self, playing: bool) {
        self.send(Cmd::Playing(playing))
    }

    /// A network screen is open, so poll often.
    pub fn watching(&self, on: bool) {
        self.send(Cmd::Watching(on))
    }

    /// Stops Wi-Fi and Syncthing so link play can have the radio. Waits until they have.
    pub fn hold(&self) {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::Hold(tx)).is_ok() {
            let _ = rx.recv_timeout(HOLD_LIMIT);
        }
    }

    pub fn release(&self) {
        self.send(Cmd::Release)
    }

    pub fn set_tailscale(&self, on: bool) {
        self.send(Cmd::SetTailscale(on))
    }

    /// Logs this device out of the tailnet; a new login link follows.
    pub fn tailscale_logout(&self) {
        self.send(Cmd::TailLogout)
    }

    /// Stops Syncthing and Wi-Fi before slot powers off or restarts. Waits a few seconds.
    pub fn shutdown(&self) {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(8));
        }
    }
}

fn lock(m: &Mutex<NetStatus>) -> MutexGuard<'_, NetStatus> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A random token from the kernel, falling back to the clock.
pub fn token(len: usize) -> String {
    const ABC: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = vec![0u8; len];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok();
    if !filled {
        let mut x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15)
            ^ u64::from(std::process::id());
        for b in &mut bytes {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
    bytes
        .iter()
        .map(|b| ABC[*b as usize % ABC.len()] as char)
        .collect()
}

struct Pending {
    net: Network,
    since: Instant,
    /// The supplicant reached the handshake, so a failure now is most likely the password.
    shook: bool,
}

struct Worker {
    root: PathBuf,
    config: NetConfig,
    wifi: Box<dyn WifiDriver>,
    sync: Box<dyn SyncEngine>,
    clock: Box<dyn Clock>,
    status: Arc<Mutex<NetStatus>>,
    country: Option<String>,
    running: bool,
    held: bool,
    playing: bool,
    watching: bool,
    pending: Option<Pending>,
    last_ssid: Option<String>,
    scan_asked: Option<Instant>,
    scan_wanted: Option<Instant>,
    sync_started: Option<Instant>,
    sync_poll_at: Option<Instant>,
    sync_paused: Option<bool>,
    sync_ready: bool,
    sync_failed: Option<String>,
    /// When the zone was last looked up, and whether that worked.
    zone_checked: Option<(Instant, bool)>,
    synced_polled: Option<Instant>,
    tail: Box<dyn TailEngine>,
    tail_started: bool,
    tail_poll_at: Option<Instant>,
    tail_failed: Option<String>,
    hostname: String,
    probe: Box<dyn Fn() -> bool + Send>,
    probe_at: Option<Instant>,
    probe_ok: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<Cmd>) {
        self.clock.set_ntp(self.config.network_time);
        if !self.config.timezone.is_empty() {
            let zone = self.config.timezone.clone();
            self.clock.set_timezone(&zone);
        }
        loop {
            self.reconcile();
            let period = if self.busy() { FAST } else { SLOW };
            match rx.recv_timeout(period) {
                Ok(Cmd::Shutdown(done)) => {
                    self.stop_tail();
                    self.stop_sync();
                    self.stop_wifi();
                    let _ = done.send(());
                    return;
                }
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.stop_tail();
                    self.stop_sync();
                    self.stop_wifi();
                    return;
                }
            }
        }
    }

    fn busy(&self) -> bool {
        self.watching
            || self.pending.is_some()
            || self.scan_asked.is_some()
            || self.scan_wanted.is_some()
            || (self.sync_started.is_some() && !self.sync_ready)
            || (self.tail_started && !matches!(lock(&self.status).tail, TailState::Running { .. }))
    }

    fn update(&self, f: impl FnOnce(&mut NetStatus)) {
        let mut s = lock(&self.status);
        let before = s.clone();
        f(&mut s);
        if *s != before {
            s.generation += 1;
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.config.save(&self.root) {
            eprintln!("slot-net: saving {}: {e}", crate::config::FILE);
        }
        let saved: Vec<String> = self
            .config
            .networks
            .iter()
            .map(|n| n.ssid.clone())
            .collect();
        let c = self.config.clone();
        self.update(|s| {
            s.saved = saved;
            s.wifi_on = c.wifi;
            s.syncthing_on = c.syncthing;
            s.sync_in_game = c.sync_in_game;
            s.network_time = c.network_time;
            s.auto_timezone = c.auto_timezone;
            s.timezone = Some(c.timezone.clone()).filter(|z| !z.is_empty());
            s.retroarch_saves = c.retroarch_saves;
            s.tailscale_on = c.tailscale;
        });
    }

    fn conf(&self) -> String {
        let mut nets = self.config.networks.clone();
        if let Some(p) = &self.pending {
            nets.retain(|n| n.ssid != p.net.ssid);
            nets.insert(0, p.net.clone());
        }
        wpa::supplicant_conf(&run_dir_str(), &nets, self.country.as_deref())
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetWifi(on) => {
                self.config.wifi = on;
                if !on {
                    self.pending = None;
                }
                self.save();
            }
            Cmd::Scan => {
                // Asked for now, sent when the supplicant is up and not busy.
                if self.config.wifi && self.scan_asked.is_none() {
                    self.scan_wanted.get_or_insert_with(Instant::now);
                    self.update(|s| s.scanning = true);
                }
            }
            Cmd::Join(net) => {
                let ssid = net.ssid.clone();
                self.pending = Some(Pending {
                    net,
                    since: Instant::now(),
                    shook: false,
                });
                self.config.wifi = true;
                self.last_ssid = None;
                self.update(|s| {
                    s.wifi_on = true;
                    s.wifi = WifiState::Joining { ssid: ssid.clone() };
                });
                self.restart_wifi(Some(&ssid));
            }
            Cmd::JoinSaved(ssid) => {
                if let Some(net) = self.config.network(&ssid).cloned() {
                    self.handle(Cmd::Join(net));
                }
            }
            Cmd::Forget(ssid) => {
                if self.config.forget(&ssid) {
                    self.save();
                    if self.pending.as_ref().is_some_and(|p| p.net.ssid == ssid) {
                        self.pending = None;
                    }
                    self.restart_wifi(None);
                }
            }
            Cmd::SetSyncthing(on) => {
                self.config.syncthing = on;
                self.sync_failed = None;
                self.save();
            }
            Cmd::SetSyncInGame(on) => {
                self.config.sync_in_game = on;
                self.save();
            }
            Cmd::SetNetworkTime(on) => {
                self.config.network_time = on;
                self.clock.set_ntp(on);
                if !on {
                    self.update(|s| s.time_synced = false);
                }
                self.synced_polled = None;
                self.save();
            }
            Cmd::SetAutoTimezone(on) => {
                self.config.auto_timezone = on;
                // Look again straight away when it is turned back on.
                self.zone_checked = None;
                self.save();
            }
            Cmd::SetRetroArchSaves(on) => {
                self.config.retroarch_saves = on;
                self.save();
            }
            Cmd::Playing(p) => self.playing = p,
            Cmd::Watching(w) => {
                self.watching = w;
                if w && self.sync_ready {
                    self.sync_poll_at = None;
                }
            }
            Cmd::Hold(done) => {
                self.held = true;
                self.stop_tail();
                self.stop_sync();
                self.stop_wifi();
                let _ = done.send(());
            }
            Cmd::Release => self.held = false,
            Cmd::SetTailscale(on) => {
                self.config.tailscale = on;
                self.tail_failed = None;
                self.save();
            }
            Cmd::TailLogout => {
                if self.tail_started {
                    self.tail.logout();
                    self.tail_poll_at = None;
                }
            }
            Cmd::Shutdown(_) => unreachable!("handled in run"),
        }
    }

    /// Rewrites the config and restarts the supplicant on it, joining `ssid` first.
    fn restart_wifi(&mut self, ssid: Option<&str>) {
        if !self.running || self.held {
            return;
        }
        let conf = self.conf();
        match self.wifi.start(&conf) {
            Ok(()) => {
                if let Some(ssid) = ssid {
                    self.wifi.select(ssid);
                }
            }
            Err(e) => self.fail_pending(&e),
        }
    }

    fn stop_wifi(&mut self) {
        if self.running {
            self.wifi.stop();
            self.running = false;
        }
        self.last_ssid = None;
        self.scan_asked = None;
        self.scan_wanted = None;
        let held = self.held;
        self.update(|s| {
            s.wifi = if held {
                WifiState::Linking
            } else {
                WifiState::Off
            };
            s.scanning = false;
        });
    }

    fn stop_tail(&mut self) {
        if std::mem::take(&mut self.tail_started) {
            self.tail.stop();
        }
        self.tail_poll_at = None;
    }

    fn stop_sync(&mut self) {
        if self.sync_started.take().is_some() {
            self.sync.stop();
        }
        self.sync_ready = false;
        self.sync_paused = None;
        self.sync_poll_at = None;
    }

    fn fail_pending(&mut self, reason: &str) {
        let Some(p) = self.pending.take() else {
            return;
        };
        let ssid = p.net.ssid.clone();
        let reason = reason.to_string();
        self.update(|s| s.wifi = WifiState::Failed { ssid, reason });
        // Back to only what was saved, so a wrong password is not retried forever.
        if self.running {
            let conf = self.conf();
            let _ = self.wifi.start(&conf);
            self.wifi.enable_all();
        }
    }

    fn reconcile(&mut self) {
        let want_wifi = self.config.wifi && !self.held;
        if want_wifi && !self.running {
            let conf = self.conf();
            match self.wifi.start(&conf) {
                Ok(()) => {
                    self.running = true;
                    if let Some(ssid) = self.pending.as_ref().map(|p| p.net.ssid.clone()) {
                        self.wifi.select(&ssid);
                    }
                }
                Err(e) => {
                    self.update(|s| {
                        s.wifi = WifiState::Failed {
                            ssid: String::new(),
                            reason: e,
                        }
                    });
                    return;
                }
            }
        } else if !want_wifi && self.running {
            self.stop_sync();
            self.stop_wifi();
        } else if !want_wifi {
            let held = self.held;
            self.update(|s| {
                s.wifi = if held {
                    WifiState::Linking
                } else {
                    WifiState::Off
                }
            });
        }
        if self.running {
            self.watch_wifi();
            self.watch_scan();
        }
        self.reconcile_online();
        self.reconcile_time();
        self.reconcile_sync();
        self.reconcile_tail();
    }

    /// Checks the internet now and then while Wi-Fi is connected.
    fn reconcile_online(&mut self) {
        let connected = self.running && lock(&self.status).ip().is_some();
        if !connected {
            self.probe_at = None;
            self.probe_ok = false;
        } else {
            let every = if self.probe_ok {
                PROBE_EVERY
            } else {
                PROBE_RETRY
            };
            if self.probe_at.is_none_or(|t| t.elapsed() > every) {
                self.probe_at = Some(Instant::now());
                self.probe_ok = (self.probe)();
            }
        }
        let online = connected && self.probe_ok;
        ONLINE.store(online, std::sync::atomic::Ordering::Relaxed);
        self.update(|s| s.online = online);
    }

    fn reconcile_tail(&mut self) {
        if !self.config.tailscale {
            self.stop_tail();
            self.update(|s| s.tail = TailState::Off);
            return;
        }
        if !self.tail.available() {
            self.update(|s| s.tail = TailState::Missing);
            return;
        }
        if let Some(e) = &self.tail_failed {
            let e = e.clone();
            self.update(|s| s.tail = TailState::Failed(e));
            return;
        }
        let connected = self.running && lock(&self.status).ip().is_some();
        if !self.tail_started {
            if !connected {
                self.update(|s| s.tail = TailState::WaitingForWifi);
                return;
            }
            match self.tail.start() {
                Ok(()) => {
                    self.tail_started = true;
                    self.tail_poll_at = None;
                    self.update(|s| s.tail = TailState::Starting);
                }
                Err(e) => {
                    eprintln!("slot-net: tailscale: {e}");
                    self.tail_failed = Some(e);
                }
            }
            return;
        }
        if !self.tail.alive() {
            self.tail_started = false;
            self.tail_failed = Some("Tailscale stopped".into());
            return;
        }
        let every = if self.watching {
            TAIL_POLL_FAST
        } else {
            TAIL_POLL_SLOW
        };
        if self.tail_poll_at.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.tail_poll_at = Some(Instant::now());
        let Some(poll) = self.tail.poll() else {
            return;
        };
        let state = tail_state(&poll);
        // Turned on means connected: ask for that, which also makes the login link.
        if matches!(poll.backend.as_str(), "NeedsLogin" | "Stopped") {
            let hostname = self.hostname.clone();
            self.tail.up(&hostname);
        }
        self.update(|s| s.tail = state);
    }

    fn reconcile_time(&mut self) {
        let online = self.running && lock(&self.status).ip().is_some();
        if !online {
            return;
        }
        if self.config.network_time && self.synced_polled.is_none_or(|t| t.elapsed() > SYNCED_POLL)
        {
            self.synced_polled = Some(Instant::now());
            let synced = self.clock.ntp_synced();
            self.update(|s| s.time_synced = synced);
        }
        if !self.config.auto_timezone {
            return;
        }
        let due = match self.zone_checked {
            None => true,
            Some((at, true)) => at.elapsed() > ZONE_EVERY,
            Some((at, false)) => at.elapsed() > ZONE_RETRY,
        };
        if !due {
            return;
        }
        let found = self.clock.lookup_zone();
        self.zone_checked = Some((Instant::now(), found.is_some()));
        let Some(zone) = found else {
            return;
        };
        if zone != self.config.timezone {
            eprintln!("slot-net: time zone is {zone}");
            self.config.timezone = zone.clone();
            self.save();
        }
        self.clock.set_timezone(&zone);
    }

    fn watch_wifi(&mut self) {
        let Some(st) = self.wifi.status() else {
            if !self.wifi.alive() {
                // The supplicant died; start it again next round.
                self.running = false;
            }
            return;
        };
        if let Some(p) = &mut self.pending {
            if st.state == "4WAY_HANDSHAKE" || st.state == "GROUP_HANDSHAKE" {
                p.shook = true;
            }
        }
        if st.completed() {
            let ssid = st.ssid.clone().unwrap_or_default();
            if self.last_ssid.as_deref() != Some(&ssid) {
                // A different network than before: the old lease means nothing here.
                if self.last_ssid.is_some() {
                    self.wifi.renew();
                }
                self.last_ssid = Some(ssid.clone());
            }
            let joined_pending = self
                .pending
                .as_ref()
                .is_some_and(|p| p.net.ssid == ssid && st.ip.is_some());
            if joined_pending {
                if let Some(p) = self.pending.take() {
                    self.config.remember(p.net);
                    self.save();
                    self.wifi.enable_all();
                }
            }
            let ip = st.ip.clone();
            if self.pending.is_none() {
                self.update(|s| s.wifi = WifiState::Connected { ssid, ip });
            }
            return;
        }
        self.last_ssid = None;
        if let Some(p) = &self.pending {
            if p.since.elapsed() > JOIN_LIMIT
                || (p.shook
                    && matches!(st.state.as_str(), "DISCONNECTED" | "SCANNING" | "INACTIVE"))
            {
                let reason = match (p.shook, p.net.psk.is_empty()) {
                    (true, false) => "Wrong password",
                    _ => "Could not connect",
                };
                self.fail_pending(reason);
            }
            return;
        }
        let state = if self.config.networks.is_empty() {
            WifiState::NoNetworks
        } else {
            WifiState::Searching
        };
        // Keep a failure on screen until something new happens.
        let failed = matches!(lock(&self.status).wifi, WifiState::Failed { .. });
        if !failed {
            self.update(|s| s.wifi = state);
        }
    }

    fn watch_scan(&mut self) {
        if let Some(wanted) = self.scan_wanted {
            if self.wifi.scan() {
                self.scan_wanted = None;
                self.scan_asked = Some(Instant::now());
            } else if wanted.elapsed() > SCAN_LIMIT {
                self.scan_wanted = None;
                self.update(|s| s.scanning = false);
            }
        }
        let Some(asked) = self.scan_asked else {
            return;
        };
        if asked.elapsed() < SCAN_SETTLE {
            return;
        }
        if let Some(results) = self.wifi.scan_results().filter(|r| !r.is_empty()) {
            self.scan_asked = None;
            self.update(|s| {
                s.scan = results;
                s.scanning = false;
            });
        } else if asked.elapsed() > SCAN_LIMIT {
            self.scan_asked = None;
            self.update(|s| s.scanning = false);
        }
    }

    fn reconcile_sync(&mut self) {
        if !self.config.syncthing {
            self.stop_sync();
            self.update(|s| {
                s.sync = SyncState::Off;
            });
            return;
        }
        if !self.sync.available() {
            self.update(|s| s.sync = SyncState::Missing);
            return;
        }
        let online = self.running && lock(&self.status).ip().is_some();
        if self.sync_started.is_none() {
            if let Some(e) = &self.sync_failed {
                let e = e.clone();
                self.update(|s| s.sync = SyncState::Failed(e));
                return;
            }
            if !online {
                self.update(|s| s.sync = SyncState::WaitingForWifi);
                return;
            }
            match self.sync.start() {
                Ok(()) => {
                    self.sync_started = Some(Instant::now());
                    self.update(|s| s.sync = SyncState::Starting);
                }
                Err(e) => {
                    eprintln!("slot-net: syncthing: {e}");
                    self.sync_failed = Some(e.clone());
                    self.update(|s| s.sync = SyncState::Failed(e));
                }
            }
            return;
        }
        if !self.sync.alive() {
            self.sync_started = None;
            self.sync_ready = false;
            self.sync_failed = Some("Syncthing stopped".into());
            return;
        }
        if !self.sync_ready {
            if self.sync.ready() {
                self.sync_ready = true;
                let id = self.sync.device_id();
                self.update(|s| s.device_id = id);
                if let Err(e) = self.sync.ensure_folders() {
                    eprintln!("slot-net: syncthing folders: {e}");
                }
            } else if self
                .sync_started
                .is_some_and(|t| t.elapsed() > SYNC_START_LIMIT)
            {
                self.stop_sync();
                self.sync_failed = Some("Syncthing did not start".into());
            }
            return;
        }
        let want_paused = self.playing && !self.config.sync_in_game;
        if self.sync_paused != Some(want_paused) && self.sync.set_paused(want_paused).is_ok() {
            if self.sync_paused == Some(true) && !want_paused {
                self.sync.scan();
            }
            self.sync_paused = Some(want_paused);
            self.sync_poll_at = None;
        }
        let every = if self.watching {
            SYNC_POLL_FAST
        } else {
            SYNC_POLL_SLOW
        };
        if self.sync_poll_at.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.sync_poll_at = Some(Instant::now());
        let poll = self.sync.poll().unwrap_or_default();
        let state = match (want_paused, poll.error) {
            (true, _) => SyncState::Paused,
            (false, Some(e)) => SyncState::Failed(e),
            (false, None) => SyncState::Running {
                devices: poll.devices,
                busy: poll.busy,
            },
        };
        self.update(|s| s.sync = state);
    }
}

fn tail_state(poll: &TailPoll) -> TailState {
    match poll.backend.as_str() {
        "Running" => TailState::Running {
            ip: poll.ip.clone(),
            name: poll.name.clone(),
            tailnet: poll.tailnet.clone(),
        },
        "NeedsLogin" => TailState::NeedsLogin {
            url: poll.auth_url.clone(),
        },
        "NeedsMachineAuth" => TailState::NeedsApproval,
        _ => TailState::Starting,
    }
}

fn run_dir_str() -> String {
    crate::run_dir().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(net: &Net, what: &str, ok: impl Fn(&NetStatus) -> bool) -> NetStatus {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let s = net.status();
            if ok(&s) {
                return s;
            }
            assert!(Instant::now() < until, "waited for {what}, last {s:?}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn start(root: &Path) -> Net {
        let net = Net::start(root, crate::sim_backends(), None);
        net.watching(true);
        net
    }

    #[test]
    fn a_fresh_card_has_wifi_on_with_nothing_saved() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        wait(&net, "no networks", |s| s.wifi == WifiState::NoNetworks);
        let cfg = NetConfig::load(d.path());
        assert_eq!(cfg.syncthing_api_key.len(), 32);
        assert_eq!(cfg.syncthing_password.len(), 10);
        net.shutdown();
    }

    #[test]
    fn scanning_lists_what_is_nearby() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        wait(&net, "radio up", |s| s.wifi == WifiState::NoNetworks);
        net.scan();
        let s = wait(&net, "results", |s| !s.scan.is_empty() && !s.scanning);
        assert_eq!(s.scan[0].ssid, "Home Wi-Fi");
        net.shutdown();
    }

    #[test]
    fn a_scan_asked_before_the_radio_is_up_still_happens() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.scan();
        let s = wait(&net, "results", |s| !s.scan.is_empty() && !s.scanning);
        assert_eq!(s.scan.len(), 4);
        net.shutdown();
    }

    #[test]
    fn joining_with_the_right_password_saves_the_network() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.join("Home Wi-Fi", "password1", false);
        let s = wait(&net, "connected", |s| {
            s.connected_ssid() == Some("Home Wi-Fi")
        });
        assert_eq!(s.ip(), Some("192.168.1.42"));
        assert_eq!(s.saved, ["Home Wi-Fi"]);
        assert_eq!(NetConfig::load(d.path()).networks[0].psk, "password1");
        net.shutdown();
    }

    #[test]
    fn a_wrong_password_is_reported_and_not_saved() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.join("Home Wi-Fi", "nope-nope", false);
        let s = wait(&net, "failure", |s| {
            matches!(s.wifi, WifiState::Failed { .. })
        });
        assert_eq!(
            s.wifi,
            WifiState::Failed {
                ssid: "Home Wi-Fi".into(),
                reason: "Wrong password".into()
            }
        );
        assert!(s.saved.is_empty());
        assert!(NetConfig::load(d.path()).networks.is_empty());
        net.shutdown();
    }

    #[test]
    fn saved_networks_join_by_themselves_on_the_next_boot() {
        let d = crate::testdir::TestDir::new("net");
        let mut c = NetConfig::default();
        c.remember(Network {
            ssid: "Home Wi-Fi".into(),
            psk: "password1".into(),
            hidden: false,
        });
        c.save(d.path()).unwrap();
        let net = start(d.path());
        wait(&net, "auto join", |s| {
            s.connected_ssid() == Some("Home Wi-Fi")
        });
        net.forget("Home Wi-Fi");
        wait(&net, "forgotten", |s| s.saved.is_empty());
        net.shutdown();
    }

    #[test]
    fn syncthing_waits_for_wifi_then_pauses_during_games() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.set_syncthing(true);
        wait(&net, "waiting", |s| s.sync == SyncState::WaitingForWifi);
        net.join("Coffee Shop", "", false);
        let s = wait(&net, "running", |s| {
            matches!(s.sync, SyncState::Running { .. })
        });
        assert_eq!(
            s.device_id.as_deref(),
            Some(crate::syncthing::SIM_DEVICE_ID)
        );
        net.playing(true);
        wait(&net, "paused", |s| s.sync == SyncState::Paused);
        net.set_sync_in_game(true);
        wait(&net, "running in game", |s| {
            matches!(s.sync, SyncState::Running { .. })
        });
        net.set_syncthing(false);
        wait(&net, "off", |s| s.sync == SyncState::Off);
        net.shutdown();
    }

    #[test]
    fn link_play_borrows_the_radio_and_gives_it_back() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.join("Coffee Shop", "", false);
        wait(&net, "connected", |s| s.connected_ssid().is_some());
        net.hold();
        assert_eq!(net.status().wifi, WifiState::Linking);
        net.release();
        wait(&net, "back", |s| s.connected_ssid() == Some("Coffee Shop"));
        net.shutdown();
    }

    #[test]
    fn turning_wifi_off_stops_it_and_is_remembered() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.set_wifi(false);
        wait(&net, "off", |s| s.wifi == WifiState::Off && !s.wifi_on);
        assert!(!NetConfig::load(d.path()).wifi);
        net.shutdown();
    }

    #[test]
    fn the_time_zone_is_found_once_online_and_remembered() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        assert_eq!(net.status().timezone, None);
        net.join("Coffee Shop", "", false);
        let s = wait(&net, "zone", |s| s.timezone.is_some());
        assert_eq!(s.timezone.as_deref(), Some("America/Los_Angeles"));
        let s = wait(&net, "synced", |s| s.time_synced);
        assert!(s.network_time);
        assert_eq!(NetConfig::load(d.path()).timezone, "America/Los_Angeles");
        net.set_network_time(false);
        net.set_auto_timezone(false);
        net.set_retroarch_saves(false);
        let s = wait(&net, "settings", |s| {
            !s.network_time && !s.auto_timezone && !s.retroarch_saves
        });
        assert!(!s.time_synced);
        let c = NetConfig::load(d.path());
        assert!(!c.network_time && !c.auto_timezone && !c.retroarch_saves);
        net.shutdown();
    }

    #[test]
    fn online_follows_the_connection() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        assert!(!net.status().online);
        net.join("Coffee Shop", "", false);
        wait(&net, "online", |s| s.online);
        net.set_wifi(false);
        wait(&net, "offline", |s| !s.online);
        net.shutdown();
    }

    #[test]
    fn tailscale_waits_for_wifi_offers_a_login_link_then_comes_up() {
        let d = crate::testdir::TestDir::new("net");
        let net = start(d.path());
        net.set_tailscale(true);
        wait(&net, "waiting", |s| s.tail == TailState::WaitingForWifi);
        assert!(NetConfig::load(d.path()).tailscale);
        net.join("Coffee Shop", "", false);
        let s = wait(&net, "link", |s| {
            matches!(&s.tail, TailState::NeedsLogin { url: Some(_) })
        });
        assert_eq!(
            s.tail,
            TailState::NeedsLogin {
                url: Some(crate::tailscale::SIM_LOGIN_URL.into())
            }
        );
        let s = wait(&net, "running", |s| {
            matches!(s.tail, TailState::Running { .. })
        });
        assert_eq!(
            s.tail,
            TailState::Running {
                ip: Some("100.101.102.103".into()),
                name: Some("slot-rg35xxh".into()),
                tailnet: Some("example.com".into()),
            }
        );
        net.tailscale_logout();
        wait(&net, "logged out", |s| {
            matches!(s.tail, TailState::NeedsLogin { .. })
        });
        net.set_tailscale(false);
        wait(&net, "off", |s| s.tail == TailState::Off && !s.tailscale_on);
        net.shutdown();
    }

    #[test]
    fn tokens_use_an_unambiguous_alphabet() {
        let t = token(64);
        assert_eq!(t.len(), 64);
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !"01ilo".contains(c)));
    }
}
