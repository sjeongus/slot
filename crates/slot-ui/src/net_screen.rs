//! Settings > Network: Wi-Fi on and off, choosing a network, typing its password with the
//! D-pad, network time, and pages of their own for Syncthing, Tailscale and RetroAchievements.
//!
//! The screen is plain state. slot hands it buttons and the latest [`NetStatus`]; it answers
//! with a [`NetIntent`] for slot to pass on to the network thread, and draws itself from text
//! faces that the frontend rasterises on demand (see [`NetFace`]).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use slot_net::{valid_passphrase, NetStatus, ScanResult, SyncState, TailState, WifiState};

use crate::draw::{Canvas, Draw, TexId};
use crate::hud::PLATE_H;
use crate::plate::{hint_coloured, title_face, Hint, UndoFace};
use crate::power_menu::{MENU_H, MENU_INK, MENU_PAD};
use crate::quick_menu::{QUICK_EDGE, QUICK_LABEL_X, QUICK_PITCH, QUICK_VISIBLE};
use crate::text;

/// The buttons the screen understands. slot maps its own input onto these.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NetKey {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    Start,
    Select,
    Menu,
}

/// What slot should do after a button press.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NetIntent {
    None,
    Close,
    SetWifi(bool),
    Scan,
    Join {
        ssid: String,
        psk: String,
        hidden: bool,
    },
    JoinSaved(String),
    Forget(String),
    SetSyncthing(bool),
    SetSyncInGame(bool),
    SetNetworkTime(bool),
    SetAutoTimezone(bool),
    SetTailscale(bool),
    TailLogout,
    SetAchievements(bool),
    AchLogin {
        user: String,
        password: String,
    },
    AchLogout,
    /// Send unlocks saved offline now.
    AchSend,
}

/// What the Achievements page shows, filled in by slot from its achievements status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AchView {
    pub enabled: bool,
    pub user: Option<String>,
    pub logged_in: bool,
    /// "Logged in", "Logging in…", or why it failed.
    pub state: String,
    /// Unlocks saved offline, waiting to be sent.
    pub pending: usize,
    /// The game being played, as "3 of 40 unlocked".
    pub game: Option<String>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum OverviewRow {
    Wifi,
    Network,
    Address,
    /// Each of these opens a page of its own.
    Syncthing,
    Tailscale,
    Achievements,
    NetworkTime,
    TimeZone,
}

impl OverviewRow {
    pub const ALL: [OverviewRow; 8] = [
        OverviewRow::Wifi,
        OverviewRow::Network,
        OverviewRow::Address,
        OverviewRow::Syncthing,
        OverviewRow::Tailscale,
        OverviewRow::Achievements,
        OverviewRow::NetworkTime,
        OverviewRow::TimeZone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OverviewRow::Wifi => "Wi-Fi",
            OverviewRow::Network => "Network",
            OverviewRow::Address => "IP Address",
            OverviewRow::Syncthing => "Syncthing",
            OverviewRow::Tailscale => "Tailscale",
            OverviewRow::Achievements => "Achievements",
            OverviewRow::NetworkTime => "Network Time",
            OverviewRow::TimeZone => "Time Zone",
        }
    }

    fn toggles(self) -> bool {
        matches!(
            self,
            OverviewRow::Wifi | OverviewRow::NetworkTime | OverviewRow::TimeZone
        )
    }

    fn opens(self) -> bool {
        matches!(
            self,
            OverviewRow::Network
                | OverviewRow::Syncthing
                | OverviewRow::Tailscale
                | OverviewRow::Achievements
        )
    }

    fn value(self, status: &NetStatus, ach: &AchView) -> Shown {
        match self {
            OverviewRow::Tailscale => Shown::Text(tail_text(status)),
            OverviewRow::Achievements => Shown::Text(ach_text(ach)),
            OverviewRow::Wifi => Shown::Switch(on_off(status.wifi_on).to_string()),
            OverviewRow::Network => Shown::Text(wifi_text(status)),
            OverviewRow::Address => Shown::Text(status.ip().unwrap_or("—").to_string()),
            OverviewRow::Syncthing => Shown::Text(sync_text(status)),
            OverviewRow::NetworkTime => Shown::Switch(time_text(status)),
            OverviewRow::TimeZone => Shown::Choice(zone_text(status)),
        }
    }
}

/// How a row's value is drawn, matching the quick menu.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Shown {
    /// ON and OFF: capitals, with arrows either side while selected.
    Switch(String),
    /// A choice in its own case (a time zone), with arrows while selected.
    Choice(String),
    /// Names, addresses and states, in their own case.
    Text(String),
}

/// The rows of the Syncthing page.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SyncRow {
    Syncthing,
    SyncInGame,
    /// Opens the device ID, QR code and web interface login.
    Details,
}

impl SyncRow {
    pub const ALL: [SyncRow; 3] = [SyncRow::Syncthing, SyncRow::SyncInGame, SyncRow::Details];

    pub fn label(self) -> &'static str {
        match self {
            SyncRow::Syncthing => "Syncthing",
            SyncRow::SyncInGame => "Sync In Games",
            SyncRow::Details => "Details",
        }
    }

    fn toggles(self) -> bool {
        matches!(self, SyncRow::Syncthing | SyncRow::SyncInGame)
    }

    fn value(self, status: &NetStatus) -> Shown {
        match self {
            SyncRow::Syncthing => Shown::Switch(on_off(status.syncthing_on).to_string()),
            SyncRow::SyncInGame => Shown::Switch(on_off(status.sync_in_game).to_string()),
            SyncRow::Details => Shown::Text(sync_text(status)),
        }
    }
}

/// The rows of the Tailscale page; which show depends on the state.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TailRow {
    Tailscale,
    Status,
    Address,
    /// This device's name on the tailnet.
    Name,
    /// Opens the login code.
    LogIn,
    LogOut,
}

impl TailRow {
    pub fn shown(status: &NetStatus) -> Vec<TailRow> {
        let mut rows = vec![TailRow::Tailscale, TailRow::Status];
        match &status.tail {
            TailState::Running { .. } => {
                rows.extend([TailRow::Address, TailRow::Name, TailRow::LogOut])
            }
            TailState::NeedsLogin { .. } => rows.push(TailRow::LogIn),
            _ => {}
        }
        rows
    }

    pub fn label(self) -> &'static str {
        match self {
            TailRow::Tailscale => "Tailscale",
            TailRow::Status => "Status",
            TailRow::Address => "Address",
            TailRow::Name => "Device Name",
            TailRow::LogIn => "Log In",
            TailRow::LogOut => "Log Out",
        }
    }

    fn value(self, status: &NetStatus) -> Shown {
        match self {
            TailRow::Tailscale => Shown::Switch(on_off(status.tailscale_on).to_string()),
            TailRow::Status => Shown::Text(tail_text(status)),
            TailRow::Address => Shown::Text(match &status.tail {
                TailState::Running { ip: Some(ip), .. } => ip.clone(),
                _ => "—".into(),
            }),
            TailRow::Name => Shown::Text(match &status.tail {
                TailState::Running {
                    name: Some(name), ..
                } => name.clone(),
                _ => "—".into(),
            }),
            TailRow::LogIn => Shown::Text("Show code".into()),
            TailRow::LogOut => Shown::Text(String::new()),
        }
    }
}

/// The rows of the Achievements page.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AchRow {
    Achievements,
    Account,
    Status,
    Game,
    /// Unlocks saved offline; A sends them now.
    Saved,
    LogOut,
}

impl AchRow {
    pub fn shown(ach: &AchView) -> Vec<AchRow> {
        let mut rows = vec![AchRow::Achievements, AchRow::Account, AchRow::Status];
        if ach.game.is_some() {
            rows.push(AchRow::Game);
        }
        rows.push(AchRow::Saved);
        if ach.logged_in {
            rows.push(AchRow::LogOut);
        }
        rows
    }

    pub fn label(self) -> &'static str {
        match self {
            AchRow::Achievements => "Achievements",
            AchRow::Account => "Account",
            AchRow::Status => "Status",
            AchRow::Game => "Game",
            AchRow::Saved => "Saved Unlocks",
            AchRow::LogOut => "Log Out",
        }
    }

    fn value(self, ach: &AchView) -> Shown {
        match self {
            AchRow::Achievements => Shown::Switch(on_off(ach.enabled).to_string()),
            AchRow::Account => Shown::Text(ach.user.clone().unwrap_or_else(|| "Log in".into())),
            AchRow::Status => Shown::Text(ach_text(ach)),
            AchRow::Game => Shown::Text(ach.game.clone().unwrap_or_default()),
            AchRow::Saved => Shown::Text(match ach.pending {
                0 => "None".into(),
                1 => "1 to send".into(),
                n => format!("{n} to send"),
            }),
            AchRow::LogOut => Shown::Text(String::new()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyTarget {
    /// The password for a network seen in a scan.
    Password {
        ssid: String,
    },
    /// The name of a network that does not broadcast it.
    HiddenName,
    /// Then its password. Left empty for an open network.
    HiddenPassword {
        ssid: String,
    },
    /// A RetroAchievements user name, then its password.
    AchUser,
    AchPassword {
        user: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NetPage {
    Overview,
    Networks,
    Keyboard(KeyTarget),
    /// Syncthing's own settings.
    Syncthing,
    /// Syncthing's device ID, QR code and web interface.
    Sync,
    Tailscale,
    /// The code and link that log this device in to Tailscale.
    TailLogin,
    Achievements,
}

/// One row on the Networks page.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetEntry {
    pub ssid: String,
    /// None for a saved network that did not show up in the last scan.
    pub bars: Option<u8>,
    pub secured: bool,
    pub sae_only: bool,
    pub saved: bool,
    pub connected: bool,
    /// The "Other Network…" row at the end.
    pub other: bool,
}

pub const OTHER_NETWORK: &str = "Other Network…";

pub fn entries(status: &NetStatus) -> Vec<NetEntry> {
    let connected = status.connected_ssid();
    let mut out: Vec<NetEntry> = status
        .scan
        .iter()
        .map(|r: &ScanResult| NetEntry {
            ssid: r.ssid.clone(),
            bars: Some(r.bars()),
            secured: r.secured,
            sae_only: r.sae_only,
            saved: status.saved.contains(&r.ssid),
            connected: connected == Some(r.ssid.as_str()),
            other: false,
        })
        .collect();
    if let Some(ssid) = connected {
        if !out.iter().any(|e| e.ssid == ssid) {
            out.insert(
                0,
                NetEntry {
                    ssid: ssid.to_string(),
                    bars: None,
                    secured: true,
                    sae_only: false,
                    saved: status.saved.iter().any(|s| s == ssid),
                    connected: true,
                    other: false,
                },
            );
        }
    }
    if let Some(at) = out.iter().position(|e| e.connected) {
        let e = out.remove(at);
        out.insert(0, e);
    }
    for ssid in &status.saved {
        if !out.iter().any(|e| &e.ssid == ssid) {
            out.push(NetEntry {
                ssid: ssid.clone(),
                bars: None,
                secured: true,
                sae_only: false,
                saved: true,
                connected: false,
                other: false,
            });
        }
    }
    out.push(NetEntry {
        ssid: OTHER_NETWORK.into(),
        bars: None,
        secured: false,
        sae_only: false,
        saved: false,
        connected: false,
        other: true,
    });
    out
}

/// The three keyboard layers.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Layer {
    Lower,
    Upper,
    Symbols,
}

const LOWER: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl@", "zxcvbnm-_."];
const UPPER: [&str; 4] = ["1234567890", "QWERTYUIOP", "ASDFGHJKL@", "ZXCVBNM-_."];
const SYMBOLS: [&str; 4] = ["!@#$%^&*()", "-_=+[]{}\\|", ";:'\",.<>/?", "`~"];

/// The bottom row of the keyboard.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Caps,
    Symbols,
    Space,
    Delete,
    Done,
}

impl Action {
    pub const ROW: [Action; 5] = [
        Action::Caps,
        Action::Symbols,
        Action::Space,
        Action::Delete,
        Action::Done,
    ];

    /// Width in key units.
    fn units(self) -> f32 {
        match self {
            Action::Space => 3.0,
            Action::Done => 2.0,
            _ => 1.5,
        }
    }

    fn text(self, layer: Layer) -> &'static str {
        match (self, layer) {
            (Action::Caps, Layer::Upper) => "abc",
            (Action::Caps, _) => "ABC",
            (Action::Symbols, Layer::Symbols) => "abc",
            (Action::Symbols, _) => "#+=",
            (Action::Space, _) => "Space",
            (Action::Delete, _) => "Delete",
            (Action::Done, _) => "Done",
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Cap {
    Char(char),
    Act(Action),
}

#[derive(Clone, Debug)]
pub struct Keyboard {
    pub text: String,
    pub layer: Layer,
    pub row: usize,
    pub col: usize,
    pub error: Option<&'static str>,
}

/// Passwords are at most 63 characters; names at most 32 bytes.
const MAX_TEXT: usize = 63;

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard {
            text: String::new(),
            layer: Layer::Lower,
            row: 1,
            col: 0,
            error: None,
        }
    }

    fn rows(&self) -> [&'static str; 4] {
        match self.layer {
            Layer::Lower => LOWER,
            Layer::Upper => UPPER,
            Layer::Symbols => SYMBOLS,
        }
    }

    pub fn row_len(&self, row: usize) -> usize {
        match row {
            0..=3 => self.rows()[row].chars().count(),
            _ => Action::ROW.len(),
        }
    }

    pub fn cap(&self, row: usize, col: usize) -> Cap {
        match row {
            0..=3 => Cap::Char(self.rows()[row].chars().nth(col).unwrap_or(' ')),
            _ => Cap::Act(Action::ROW[col.min(Action::ROW.len() - 1)]),
        }
    }

    pub fn selected(&self) -> Cap {
        self.cap(self.row, self.col)
    }

    /// Key centres in key units, so moving up or down lands on the key nearest above or below.
    fn centre(&self, row: usize, col: usize) -> f32 {
        match row {
            0..=3 => {
                let n = self.row_len(row) as f32;
                (10.0 - n) / 2.0 + col as f32 + 0.5
            }
            _ => {
                let before: f32 = Action::ROW[..col].iter().map(|a| a.units()).sum();
                let total: f32 = Action::ROW.iter().map(|a| a.units()).sum();
                (10.0 - total) / 2.0 + before + Action::ROW[col].units() / 2.0
            }
        }
    }

    fn nearest(&self, row: usize, x: f32) -> usize {
        (0..self.row_len(row))
            .min_by(|&a, &b| {
                (self.centre(row, a) - x)
                    .abs()
                    .total_cmp(&(self.centre(row, b) - x).abs())
            })
            .unwrap_or(0)
    }

    pub fn move_by(&mut self, key: NetKey) {
        let x = self.centre(self.row, self.col);
        match key {
            NetKey::Left => {
                let n = self.row_len(self.row);
                self.col = (self.col + n - 1) % n;
            }
            NetKey::Right => self.col = (self.col + 1) % self.row_len(self.row),
            NetKey::Up => {
                self.row = (self.row + 4) % 5;
                self.col = self.nearest(self.row, x);
            }
            NetKey::Down => {
                self.row = (self.row + 1) % 5;
                self.col = self.nearest(self.row, x);
            }
            _ => {}
        }
    }

    pub fn type_char(&mut self, c: char) {
        if self.text.chars().count() < MAX_TEXT {
            self.text.push(c);
        }
        self.error = None;
    }

    pub fn delete(&mut self) {
        self.text.pop();
        self.error = None;
    }

    pub fn toggle_caps(&mut self) {
        self.layer = match self.layer {
            Layer::Upper => Layer::Lower,
            _ => Layer::Upper,
        };
        self.col = self.col.min(self.row_len(self.row) - 1);
    }

    pub fn toggle_symbols(&mut self) {
        self.layer = match self.layer {
            Layer::Symbols => Layer::Lower,
            _ => Layer::Symbols,
        };
        self.col = self.col.min(self.row_len(self.row) - 1);
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NetScreen {
    pub page: NetPage,
    pub row: usize,
    /// The highlighted row on the Syncthing page.
    pub sync_row: usize,
    /// The highlighted row on the Tailscale and Achievements pages.
    pub tail_row: usize,
    pub ach_row: usize,
    pub pick: usize,
    pub keyboard: Keyboard,
    /// The achievements status, which slot keeps up to date.
    pub ach: AchView,
}

impl Default for NetScreen {
    fn default() -> Self {
        Self::new()
    }
}

/// Switches read in capitals, as on slot's other menus; names and addresses keep their case.
fn on_off(on: bool) -> &'static str {
    if on {
        "ON"
    } else {
        "OFF"
    }
}

pub fn wifi_text(status: &NetStatus) -> String {
    match &status.wifi {
        WifiState::Off => "OFF".into(),
        WifiState::Linking => "In use by link".into(),
        WifiState::NoNetworks => "Choose a network".into(),
        WifiState::Searching => "Searching…".into(),
        WifiState::Joining { ssid } => format!("Joining {ssid}…"),
        WifiState::Connected { ssid, .. } => ssid.clone(),
        WifiState::Failed { ssid, reason } if ssid.is_empty() => reason.clone(),
        WifiState::Failed { ssid, reason } => format!("{ssid}: {reason}"),
    }
}

pub fn time_text(status: &NetStatus) -> String {
    match (status.network_time, status.time_synced) {
        (false, _) => "OFF".into(),
        (true, true) => "ON, synced".into(),
        (true, false) => "ON".into(),
    }
}

pub fn zone_text(status: &NetStatus) -> String {
    match (status.auto_timezone, &status.timezone) {
        (false, _) => "Manual".into(),
        (true, Some(z)) => format!("Auto: {}", slot_net::time::city(z)),
        (true, None) => "Auto: finding…".into(),
    }
}

pub fn tail_text(status: &NetStatus) -> String {
    match &status.tail {
        TailState::Off => "OFF".into(),
        TailState::Missing => "Not on this card".into(),
        TailState::WaitingForWifi => "Waiting for Wi-Fi".into(),
        TailState::Starting => "Starting…".into(),
        TailState::NeedsLogin { .. } => "Log in to connect".into(),
        TailState::NeedsApproval => "Waiting for approval".into(),
        TailState::Running { .. } => "Connected".into(),
        TailState::Failed(e) => e.clone(),
    }
}

pub fn ach_text(ach: &AchView) -> String {
    match ach.enabled {
        false => "OFF".into(),
        true => ach.state.clone(),
    }
}

pub fn sync_text(status: &NetStatus) -> String {
    match &status.sync {
        SyncState::Off => "OFF".into(),
        SyncState::Missing => "Not on this card".into(),
        SyncState::WaitingForWifi => "Waiting for Wi-Fi".into(),
        SyncState::Starting => "Starting…".into(),
        SyncState::Paused => "Paused while playing".into(),
        SyncState::Running { busy: true, .. } => "Syncing…".into(),
        SyncState::Running { devices: 0, .. } => "No devices connected".into(),
        SyncState::Running { devices: 1, .. } => "1 device connected".into(),
        SyncState::Running { devices, .. } => format!("{devices} devices connected"),
        SyncState::Failed(e) => e.clone(),
    }
}

impl NetScreen {
    pub fn new() -> NetScreen {
        NetScreen {
            page: NetPage::Overview,
            row: 0,
            sync_row: 0,
            tail_row: 0,
            ach_row: 0,
            pick: 0,
            keyboard: Keyboard::new(),
            ach: AchView::default(),
        }
    }

    pub fn tail_row(&self, status: &NetStatus) -> TailRow {
        let rows = TailRow::shown(status);
        rows[self.tail_row.min(rows.len() - 1)]
    }

    pub fn ach_row(&self) -> AchRow {
        let rows = AchRow::shown(&self.ach);
        rows[self.ach_row.min(rows.len() - 1)]
    }

    pub fn overview_row(&self) -> OverviewRow {
        OverviewRow::ALL[self.row.min(OverviewRow::ALL.len() - 1)]
    }

    pub fn sync_row(&self) -> SyncRow {
        SyncRow::ALL[self.sync_row.min(SyncRow::ALL.len() - 1)]
    }

    pub fn input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        match self.page.clone() {
            NetPage::Overview => self.overview_input(key, status),
            NetPage::Networks => self.networks_input(key, status),
            NetPage::Keyboard(target) => self.keyboard_input(key, target),
            NetPage::Syncthing => self.syncthing_input(key, status),
            NetPage::Sync => match key {
                NetKey::B | NetKey::Menu => {
                    self.page = NetPage::Syncthing;
                    NetIntent::None
                }
                _ => NetIntent::None,
            },
            NetPage::Tailscale => self.tailscale_input(key, status),
            NetPage::TailLogin => match key {
                NetKey::B | NetKey::Menu => {
                    self.page = NetPage::Tailscale;
                    NetIntent::None
                }
                _ => NetIntent::None,
            },
            NetPage::Achievements => self.achievements_input(key),
        }
    }

    fn tailscale_input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        let n = TailRow::shown(status).len();
        self.tail_row = self.tail_row.min(n - 1);
        let row = self.tail_row(status);
        match key {
            NetKey::Up => {
                self.tail_row = (self.tail_row + n - 1) % n;
                NetIntent::None
            }
            NetKey::Down => {
                self.tail_row = (self.tail_row + 1) % n;
                NetIntent::None
            }
            NetKey::Left | NetKey::Right | NetKey::A if row == TailRow::Tailscale => {
                NetIntent::SetTailscale(!status.tailscale_on)
            }
            NetKey::A if row == TailRow::LogIn => {
                self.page = NetPage::TailLogin;
                NetIntent::None
            }
            NetKey::A if row == TailRow::LogOut => {
                self.tail_row = 0;
                NetIntent::TailLogout
            }
            NetKey::B | NetKey::Menu => {
                self.page = NetPage::Overview;
                NetIntent::None
            }
            _ => NetIntent::None,
        }
    }

    fn achievements_input(&mut self, key: NetKey) -> NetIntent {
        let n = AchRow::shown(&self.ach).len();
        self.ach_row = self.ach_row.min(n - 1);
        let row = self.ach_row();
        match key {
            NetKey::Up => {
                self.ach_row = (self.ach_row + n - 1) % n;
                NetIntent::None
            }
            NetKey::Down => {
                self.ach_row = (self.ach_row + 1) % n;
                NetIntent::None
            }
            NetKey::Left | NetKey::Right | NetKey::A if row == AchRow::Achievements => {
                NetIntent::SetAchievements(!self.ach.enabled)
            }
            NetKey::A if row == AchRow::Account => {
                self.open_keyboard(KeyTarget::AchUser);
                if let Some(user) = &self.ach.user {
                    self.keyboard.text = user.clone();
                }
                NetIntent::None
            }
            NetKey::A if row == AchRow::Saved && self.ach.pending > 0 => NetIntent::AchSend,
            NetKey::A if row == AchRow::LogOut => {
                self.ach_row = 1;
                NetIntent::AchLogout
            }
            NetKey::B | NetKey::Menu => {
                self.page = NetPage::Overview;
                NetIntent::None
            }
            _ => NetIntent::None,
        }
    }

    fn back_to_achievements(&mut self) {
        self.page = NetPage::Achievements;
        self.ach_row = 1;
    }

    fn syncthing_input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        let n = SyncRow::ALL.len();
        let row = self.sync_row();
        let toggle = |row: SyncRow| match row {
            SyncRow::Syncthing => NetIntent::SetSyncthing(!status.syncthing_on),
            SyncRow::SyncInGame => NetIntent::SetSyncInGame(!status.sync_in_game),
            SyncRow::Details => NetIntent::None,
        };
        match key {
            NetKey::Up => {
                self.sync_row = (self.sync_row + n - 1) % n;
                NetIntent::None
            }
            NetKey::Down => {
                self.sync_row = (self.sync_row + 1) % n;
                NetIntent::None
            }
            NetKey::Left | NetKey::Right if row.toggles() => toggle(row),
            NetKey::A if row.toggles() => toggle(row),
            NetKey::A => {
                self.page = NetPage::Sync;
                NetIntent::None
            }
            NetKey::B | NetKey::Menu => {
                self.page = NetPage::Overview;
                NetIntent::None
            }
            _ => NetIntent::None,
        }
    }

    fn open_networks(&mut self) -> NetIntent {
        self.page = NetPage::Networks;
        self.pick = 0;
        NetIntent::Scan
    }

    fn overview_input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        let n = OverviewRow::ALL.len();
        let row = self.overview_row();
        let toggle = |row: OverviewRow| match row {
            OverviewRow::Wifi => NetIntent::SetWifi(!status.wifi_on),
            OverviewRow::NetworkTime => NetIntent::SetNetworkTime(!status.network_time),
            OverviewRow::TimeZone => NetIntent::SetAutoTimezone(!status.auto_timezone),
            _ => NetIntent::None,
        };
        match key {
            NetKey::Up => {
                self.row = (self.row + n - 1) % n;
                NetIntent::None
            }
            NetKey::Down => {
                self.row = (self.row + 1) % n;
                NetIntent::None
            }
            NetKey::Left | NetKey::Right if row.toggles() => toggle(row),
            NetKey::A => match row {
                r if r.toggles() => toggle(r),
                OverviewRow::Network if status.wifi_on => self.open_networks(),
                OverviewRow::Network => {
                    // Choosing a network implies wanting Wi-Fi.
                    self.page = NetPage::Networks;
                    self.pick = 0;
                    NetIntent::SetWifi(true)
                }
                OverviewRow::Syncthing => {
                    self.page = NetPage::Syncthing;
                    self.sync_row = 0;
                    NetIntent::None
                }
                OverviewRow::Tailscale => {
                    self.page = NetPage::Tailscale;
                    self.tail_row = 0;
                    NetIntent::None
                }
                OverviewRow::Achievements => {
                    self.page = NetPage::Achievements;
                    self.ach_row = 0;
                    NetIntent::None
                }
                _ => NetIntent::None,
            },
            NetKey::B | NetKey::Menu => NetIntent::Close,
            _ => NetIntent::None,
        }
    }

    fn networks_input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        let list = entries(status);
        self.pick = self.pick.min(list.len() - 1);
        let n = list.len();
        let e = &list[self.pick];
        match key {
            NetKey::Up => {
                self.pick = (self.pick + n - 1) % n;
                NetIntent::None
            }
            NetKey::Down => {
                self.pick = (self.pick + 1) % n;
                NetIntent::None
            }
            NetKey::X => NetIntent::Scan,
            NetKey::Y if e.saved => NetIntent::Forget(e.ssid.clone()),
            NetKey::A if e.other => {
                self.open_keyboard(KeyTarget::HiddenName);
                NetIntent::None
            }
            NetKey::A if e.connected => NetIntent::None,
            NetKey::A if e.saved => {
                self.back_to_network_row();
                NetIntent::JoinSaved(e.ssid.clone())
            }
            NetKey::A if !e.secured => {
                self.back_to_network_row();
                NetIntent::Join {
                    ssid: e.ssid.clone(),
                    psk: String::new(),
                    hidden: false,
                }
            }
            NetKey::A => {
                self.open_keyboard(KeyTarget::Password {
                    ssid: e.ssid.clone(),
                });
                NetIntent::None
            }
            NetKey::B | NetKey::Menu => {
                self.page = NetPage::Overview;
                NetIntent::None
            }
            _ => NetIntent::None,
        }
    }

    fn back_to_network_row(&mut self) {
        self.page = NetPage::Overview;
        self.row = OverviewRow::ALL
            .iter()
            .position(|r| *r == OverviewRow::Network)
            .unwrap_or(0);
    }

    fn open_keyboard(&mut self, target: KeyTarget) {
        self.keyboard = Keyboard::new();
        self.page = NetPage::Keyboard(target);
    }

    fn keyboard_input(&mut self, key: NetKey, target: KeyTarget) -> NetIntent {
        let kb = &mut self.keyboard;
        match key {
            NetKey::Up | NetKey::Down | NetKey::Left | NetKey::Right => {
                kb.move_by(key);
                NetIntent::None
            }
            NetKey::A => match kb.selected() {
                Cap::Char(c) => {
                    kb.type_char(c);
                    NetIntent::None
                }
                Cap::Act(Action::Caps) => {
                    kb.toggle_caps();
                    NetIntent::None
                }
                Cap::Act(Action::Symbols) => {
                    kb.toggle_symbols();
                    NetIntent::None
                }
                Cap::Act(Action::Space) => {
                    kb.type_char(' ');
                    NetIntent::None
                }
                Cap::Act(Action::Delete) => {
                    kb.delete();
                    NetIntent::None
                }
                Cap::Act(Action::Done) => self.finish(target),
            },
            NetKey::X => {
                kb.type_char(' ');
                NetIntent::None
            }
            NetKey::Y => {
                kb.toggle_caps();
                NetIntent::None
            }
            NetKey::Select => {
                kb.toggle_symbols();
                NetIntent::None
            }
            NetKey::B if !kb.text.is_empty() => {
                kb.delete();
                NetIntent::None
            }
            NetKey::B | NetKey::Menu => {
                self.page = match target {
                    KeyTarget::AchUser | KeyTarget::AchPassword { .. } => NetPage::Achievements,
                    _ => NetPage::Networks,
                };
                NetIntent::None
            }
            NetKey::Start => self.finish(target),
        }
    }

    fn finish(&mut self, target: KeyTarget) -> NetIntent {
        let text = self.keyboard.text.clone();
        match target {
            KeyTarget::AchUser => {
                if text.trim().is_empty() {
                    self.keyboard.error = Some("Type your RetroAchievements user name");
                    return NetIntent::None;
                }
                self.open_keyboard(KeyTarget::AchPassword {
                    user: text.trim().to_string(),
                });
                NetIntent::None
            }
            KeyTarget::AchPassword { user } => {
                if text.is_empty() {
                    self.keyboard.error = Some("Type the account's password");
                    return NetIntent::None;
                }
                self.back_to_achievements();
                NetIntent::AchLogin {
                    user,
                    password: text,
                }
            }
            KeyTarget::HiddenName => {
                if text.trim().is_empty() || text.len() > 32 {
                    self.keyboard.error = Some("Names are 1 to 32 characters");
                    return NetIntent::None;
                }
                self.open_keyboard(KeyTarget::HiddenPassword { ssid: text });
                NetIntent::None
            }
            KeyTarget::HiddenPassword { ssid } if text.is_empty() => {
                self.back_to_network_row();
                NetIntent::Join {
                    ssid,
                    psk: String::new(),
                    hidden: true,
                }
            }
            KeyTarget::Password { ssid } | KeyTarget::HiddenPassword { ssid } => {
                if !valid_passphrase(&text) {
                    self.keyboard.error = Some("Passwords are 8 to 63 characters");
                    return NetIntent::None;
                }
                let hidden = matches!(
                    self.page,
                    NetPage::Keyboard(KeyTarget::HiddenPassword { .. })
                );
                self.back_to_network_row();
                NetIntent::Join {
                    ssid,
                    psk: text,
                    hidden,
                }
            }
        }
    }

    pub fn title(&self) -> &'static str {
        match &self.page {
            NetPage::Overview => "Network",
            NetPage::Networks => "Wi-Fi Networks",
            NetPage::Keyboard(KeyTarget::HiddenName) => "Network Name",
            NetPage::Keyboard(KeyTarget::AchUser) => "User Name",
            NetPage::Keyboard(_) => "Password",
            NetPage::Syncthing => "Syncthing",
            NetPage::Sync => "Syncthing Details",
            NetPage::Tailscale => "Tailscale",
            NetPage::TailLogin => "Log In to Tailscale",
            NetPage::Achievements => "Achievements",
        }
    }

    pub fn hints(&self, status: &NetStatus) -> (Vec<Hint>, usize) {
        let h = |key: &'static str, label: &str| Hint {
            key,
            label: label.to_string(),
        };
        match &self.page {
            NetPage::Overview => {
                let row = self.overview_row();
                let a = match row {
                    r if r.toggles() => Some("Toggle"),
                    r if r.opens() => Some("Open"),
                    _ => None,
                };
                let mut v = vec![h("B", "Back")];
                v.extend(a.map(|a| h("A", a)));
                (v, 1)
            }
            NetPage::Networks => {
                let list = entries(status);
                let e = &list[self.pick.min(list.len() - 1)];
                let mut v = vec![h("B", "Back")];
                if e.saved {
                    v.push(h("Y", "Forget"));
                }
                v.push(h("X", "Rescan"));
                if !e.connected {
                    v.push(h("A", "Join"));
                }
                (v, 1)
            }
            NetPage::Keyboard(_) => (
                vec![
                    h("B", "Delete"),
                    h("Y", "Caps"),
                    h("SELECT", "#+="),
                    h("START", "Done"),
                ],
                1,
            ),
            NetPage::Syncthing => {
                let a = match self.sync_row().toggles() {
                    true => "Toggle",
                    false => "Open",
                };
                (vec![h("B", "Back"), h("A", a)], 1)
            }
            NetPage::Sync | NetPage::TailLogin => (vec![h("B", "Back")], 1),
            NetPage::Tailscale => {
                let a = match self.tail_row(status) {
                    TailRow::Tailscale => Some("Toggle"),
                    TailRow::LogIn => Some("Open"),
                    TailRow::LogOut => Some("Log Out"),
                    _ => None,
                };
                let mut v = vec![h("B", "Back")];
                v.extend(a.map(|a| h("A", a)));
                (v, 1)
            }
            NetPage::Achievements => {
                let a = match self.ach_row() {
                    AchRow::Achievements => Some("Toggle"),
                    AchRow::Account => Some("Log In"),
                    AchRow::Saved if self.ach.pending > 0 => Some("Send Now"),
                    AchRow::LogOut => Some("Log Out"),
                    _ => None,
                };
                let mut v = vec![h("B", "Back")];
                v.extend(a.map(|a| h("A", a)));
                (v, 1)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Faces

/// A piece of text or art the screen draws. The frontend rasterises each one once, through
/// [`net_face`], and hands back a texture.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum NetFace {
    Title(String),
    /// Row labels and switch values, drawn exactly as the quick menu draws its own.
    Label(String, bool),
    /// A row's value in its own case, at the quick menu's size.
    Row(String, bool),
    /// The quick menu's pointer and its left and right arrows.
    Pointer,
    Caret(bool),
    /// Values and network names, case kept.
    Value(String, bool),
    /// Secondary lines, case kept.
    Small(String),
    /// Device IDs, addresses and passwords.
    Mono(String),
    Key(String, bool),
    Field(String),
    Hint(&'static str, String),
    Lock(bool),
    Qr(String),
    /// An achievement badge from the card.
    Badge(std::path::PathBuf),
}

const DIM: [u8; 3] = [0x9a, 0x9a, 0xa4];
const DARK: [u8; 3] = [0x1a, 0x1a, 0x1e];
const ROW_MAX_W: u32 = 340;
const VALUE_PX: f32 = 22.0;
const SMALL_PX: f32 = 17.0;
const MONO_PX: f32 = 19.0;
const KEY_PX: f32 = 22.0;
const FIELD_PX: f32 = 26.0;
const MAX_FACE_W: u32 = 600;
const QR_SCALE: u32 = 4;
const QR_SIDE: u32 = 200;

fn empty() -> UndoFace {
    UndoFace {
        rgba: Vec::new(),
        w: 0,
        h: 0,
    }
}

/// One line of text in its own case, cut short with an ellipsis past `max_w`.
pub fn plain_face(line: &str, px: f32, ink: [u8; 3], max_w: u32) -> UndoFace {
    let Some(font) = text::label_font() else {
        return empty();
    };
    let Some(vm) = font.horizontal_line_metrics(px) else {
        return empty();
    };
    let width = |s: &str| text::line_width(font, s, px, 0.0);
    let mut shown: String = line.to_string();
    if width(&shown) > max_w as f32 {
        while !shown.is_empty() && width(&format!("{shown}…")) > max_w as f32 {
            shown.pop();
        }
        shown.push('…');
    }
    let w = (width(&shown).ceil() as u32 + 2).max(1);
    let h = vm.new_line_size.ceil() as u32 + 2;
    let mut cov = vec![0u8; (w * h) as usize];
    let baseline = 1.0 + vm.ascent;
    let mut pen = 1.0f32;
    for ch in shown.chars() {
        let (m, glyph) = font.rasterize(ch, px);
        let x0 = (pen + m.xmin as f32).round() as i32;
        let y0 = (baseline - (m.height as f32 + m.ymin as f32)).round() as i32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let (x, y) = (x0 + gx as i32, y0 + gy as i32);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let at = (y as u32 * w + x as u32) as usize;
                cov[at] = cov[at].max(glyph[gy * m.width + gx]);
            }
        }
        pen += m.advance_width;
    }
    tint(&cov, w, h, ink)
}

fn tint(cov: &[u8], w: u32, h: u32, ink: [u8; 3]) -> UndoFace {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (i, a) in cov.iter().enumerate() {
        rgba[i * 4..i * 4 + 4].copy_from_slice(&[ink[0], ink[1], ink[2], *a]);
    }
    UndoFace { rgba, w, h }
}

/// A value in its own case, at the quick menu's size, in a box of the quick menu's height
/// with the same side padding, so it lines up with the capitals beside it.
fn row_face(line: &str, lit: bool) -> UndoFace {
    let ink = if lit { MENU_INK } else { DIM };
    let face = plain_face(line, crate::power_menu::MENU_PX, ink, ROW_MAX_W);
    let (pad, h) = (MENU_PAD, MENU_H);
    let w = face.w + 2 * pad;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let top = h.saturating_sub(face.h) / 2;
    crate::plate::blit(&mut rgba, w, &face.rgba, face.w, face.h.min(h), pad, top);
    UndoFace { rgba, w, h }
}

/// A padlock, drawn as pixels: the symbols font on the card is a subset without one.
const LOCK: [&str; 9] = [
    "..XXXX..", ".X....X.", ".X....X.", "XXXXXXXX", "XXXXXXXX", "XXX..XXX", "XXX..XXX", "XXXXXXXX",
    "XXXXXXXX",
];
const LOCK_SCALE: u32 = 2;

fn lock_face(lit: bool) -> UndoFace {
    let ink = if lit { MENU_INK } else { DIM };
    let (w, h) = (8 * LOCK_SCALE, LOCK.len() as u32 * LOCK_SCALE);
    let mut cov = vec![0u8; (w * h) as usize];
    for (y, row) in LOCK.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if ch != 'X' {
                continue;
            }
            for py in 0..LOCK_SCALE {
                for px in 0..LOCK_SCALE {
                    let at =
                        ((y as u32 * LOCK_SCALE + py) * w + x as u32 * LOCK_SCALE + px) as usize;
                    cov[at] = 0xff;
                }
            }
        }
    }
    tint(&cov, w, h, ink)
}

/// The QR code of `data`, black on white with a quiet zone, ready to scan from a phone.
pub fn qr_face(data: &str) -> UndoFace {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M) else {
        return empty();
    };
    let n = code.width() as u32;
    let quiet = 4;
    // Bigger when the data is short, so a short link scans from further away.
    let scale = (QR_SIDE / (n + 2 * quiet)).clamp(QR_SCALE, 8);
    let side = (n + 2 * quiet) * scale;
    let mut rgba = vec![0xffu8; (side * side * 4) as usize];
    let colors = code.to_colors();
    for my in 0..n {
        for mx in 0..n {
            if colors[(my * n + mx) as usize] != qrcode::Color::Dark {
                continue;
            }
            for py in 0..scale {
                for px in 0..scale {
                    let x = (mx + quiet) * scale + px;
                    let y = (my + quiet) * scale + py;
                    let at = ((y * side + x) * 4) as usize;
                    rgba[at..at + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    UndoFace {
        rgba,
        w: side,
        h: side,
    }
}

pub fn net_face(face: &NetFace) -> UndoFace {
    let ink = |lit: bool| if lit { MENU_INK } else { DIM };
    match face {
        NetFace::Title(t) => title_face(t),
        NetFace::Label(t, lit) => crate::quick_menu::quick_value_face(t, *lit),
        NetFace::Row(t, lit) => row_face(t, *lit),
        NetFace::Pointer => crate::quick_menu::quick_pointer_face(),
        NetFace::Caret(right) => crate::quick_menu::quick_caret_face(*right),
        NetFace::Value(t, lit) => plain_face(t, VALUE_PX, ink(*lit), 340),
        NetFace::Small(t) => plain_face(t, SMALL_PX, DIM, MAX_FACE_W),
        NetFace::Mono(t) => plain_face(t, MONO_PX, MENU_INK, MAX_FACE_W),
        NetFace::Key(t, lit) => plain_face(t, KEY_PX, if *lit { DARK } else { MENU_INK }, 160),
        NetFace::Field(t) => plain_face(t, FIELD_PX, MENU_INK, 560),
        NetFace::Hint(key, label) => hint_coloured(key, label, MENU_INK),
        NetFace::Lock(lit) => lock_face(*lit),
        NetFace::Qr(data) => qr_face(data),
        NetFace::Badge(path) => crate::notice::achievement_badge_face(path),
    }
}

/// Textures for faces, filled in by the frontend. Draw notes what it wanted, so the frontend
/// knows what to build and what it may recycle.
#[derive(Default)]
pub struct NetFaces {
    pub ready: HashMap<NetFace, (TexId, u32, u32)>,
    wanted: RefCell<Vec<NetFace>>,
    used: RefCell<HashSet<NetFace>>,
}

impl NetFaces {
    pub fn get(&self, face: NetFace) -> Option<(TexId, u32, u32)> {
        match self.ready.get(&face) {
            Some(t) => {
                self.used.borrow_mut().insert(face);
                Some(*t)
            }
            None => {
                let mut w = self.wanted.borrow_mut();
                if !w.contains(&face) {
                    w.push(face);
                }
                None
            }
        }
    }

    /// Faces the last draw asked for and did not have.
    pub fn take_wanted(&self) -> Vec<NetFace> {
        std::mem::take(&mut self.wanted.borrow_mut())
    }

    /// Faces the last draw used. Everything else can be recycled.
    pub fn take_used(&self) -> HashSet<NetFace> {
        std::mem::take(&mut self.used.borrow_mut())
    }
}

// ---------------------------------------------------------------------------------------
// Layout and drawing

const EDGE: f32 = 40.0;
const LIST_PITCH: f32 = 46.0;
const LIST_SHOWN: usize = 7;
const HIGHLIGHT: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
const KEY_FILL: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
const KEY_LIT: [f32; 4] = [0.965, 0.957, 0.937, 1.0];
const FIELD_FILL: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const BAR_ON: [f32; 4] = [0.965, 0.957, 0.937, 1.0];
const BAR_OFF: [f32; 4] = [1.0, 1.0, 1.0, 0.18];
const KEY_GAP: f32 = 7.0;
/// The quick menu's own spacing, so the two read as one menu.
const TYPE_DROP: f32 = 4.0;
const POINTER_GAP: f32 = 14.0;
const POINTER_BLEED: f32 = 4.0;
const CARET_GAP: f32 = 14.0;
const SCROLL_INSET: f32 = 12.0;
const SCROLL_W: f32 = 4.0;
const KEY_H: f32 = 44.0;

fn tex(out: &mut Vec<Draw>, t: (TexId, u32, u32), x: f32, y: f32) {
    out.push(Draw::Tex {
        x: x.round(),
        y: y.round(),
        w: t.1 as f32,
        h: t.2 as f32,
        tex: t.0,
        alpha: 1.0,
    });
}

fn rect(out: &mut Vec<Draw>, x: f32, y: f32, w: f32, h: f32, colour: [f32; 4]) {
    out.push(Draw::Rect { x, y, w, h, colour });
}

/// The geometry of one keyboard key, for drawing and for tests.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct KeyBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn key_unit(canvas: Canvas) -> f32 {
    ((canvas.w - 2.0 * 24.0 + KEY_GAP) / 10.0).min(60.0)
}

pub fn key_box(kb: &Keyboard, canvas: Canvas, top: f32, row: usize, col: usize) -> KeyBox {
    let unit = key_unit(canvas);
    let left = (canvas.w - unit * 10.0) / 2.0;
    let y = top + row as f32 * (KEY_H + KEY_GAP);
    let (x, units) = match row {
        0..=3 => {
            let n = kb.row_len(row) as f32;
            (left + ((10.0 - n) / 2.0 + col as f32) * unit, 1.0)
        }
        _ => {
            let total: f32 = Action::ROW.iter().map(|a| a.units()).sum();
            let before: f32 = Action::ROW[..col].iter().map(|a| a.units()).sum();
            (
                left + ((10.0 - total) / 2.0 + before) * unit,
                Action::ROW[col].units(),
            )
        }
    };
    KeyBox {
        x: x.round(),
        y,
        w: (units * unit - KEY_GAP).round(),
        h: KEY_H,
    }
}

impl NetScreen {
    /// Draws the page's body; the frame around it (title and hint plates) is drawn by
    /// [`draw_net_chrome`].
    pub fn draw(&self, canvas: Canvas, status: &NetStatus, faces: &NetFaces, out: &mut Vec<Draw>) {
        match &self.page {
            NetPage::Overview => {
                let rows: Vec<(&str, Shown)> = OverviewRow::ALL
                    .iter()
                    .map(|r| (r.label(), r.value(status, &self.ach)))
                    .collect();
                draw_rows(canvas, &rows, self.row, faces, out);
            }
            NetPage::Syncthing => {
                let rows: Vec<(&str, Shown)> = SyncRow::ALL
                    .iter()
                    .map(|r| (r.label(), r.value(status)))
                    .collect();
                draw_rows(canvas, &rows, self.sync_row, faces, out);
            }
            NetPage::Networks => self.draw_networks(canvas, status, faces, out),
            NetPage::Keyboard(target) => self.draw_keyboard(canvas, target, faces, out),
            NetPage::Sync => draw_sync(canvas, status, faces, out),
            NetPage::Tailscale => {
                let rows: Vec<(&str, Shown)> = TailRow::shown(status)
                    .iter()
                    .map(|r| (r.label(), r.value(status)))
                    .collect();
                draw_rows(canvas, &rows, self.tail_row, faces, out);
            }
            NetPage::TailLogin => draw_tail_login(canvas, status, faces, out),
            NetPage::Achievements => {
                let rows: Vec<(&str, Shown)> = AchRow::shown(&self.ach)
                    .iter()
                    .map(|r| (r.label(), r.value(&self.ach)))
                    .collect();
                draw_rows(canvas, &rows, self.ach_row, faces, out);
            }
        }
    }

    fn draw_networks(
        &self,
        canvas: Canvas,
        status: &NetStatus,
        faces: &NetFaces,
        out: &mut Vec<Draw>,
    ) {
        let list = entries(status);
        let pick = self.pick.min(list.len() - 1);
        let first = pick
            .saturating_sub(LIST_SHOWN / 2)
            .min(list.len().saturating_sub(LIST_SHOWN));
        let top = PLATE_H + 14.0;
        if status.scanning || (status.scan.is_empty() && status.wifi_on) {
            let note = if status.scanning {
                "Looking for networks…"
            } else {
                "Press X to look again"
            };
            if let Some(t) = faces.get(NetFace::Small(note.into())) {
                tex(
                    out,
                    t,
                    canvas.centred(t.1 as f32),
                    canvas.h - PLATE_H - t.2 as f32 - 8.0,
                );
            }
        }
        for (slot, (i, e)) in list
            .iter()
            .enumerate()
            .skip(first)
            .take(LIST_SHOWN)
            .enumerate()
        {
            let y = top + slot as f32 * LIST_PITCH;
            let lit = i == pick;
            if lit {
                rect(
                    out,
                    EDGE / 2.0,
                    y,
                    canvas.w - EDGE,
                    LIST_PITCH - 4.0,
                    HIGHLIGHT,
                );
            }
            let mid = y + (LIST_PITCH - 4.0) / 2.0;
            let mut x = EDGE;
            if e.secured && !e.other {
                if let Some(t) = faces.get(NetFace::Lock(lit)) {
                    tex(out, t, x, mid - t.2 as f32 / 2.0);
                }
            }
            x += 26.0;
            if let Some(t) = faces.get(NetFace::Value(e.ssid.clone(), lit)) {
                tex(out, t, x, mid - t.2 as f32 / 2.0);
            }
            let mut right = canvas.w - EDGE;
            if let Some(bars) = e.bars {
                right -= 30.0;
                draw_bars(out, right, mid, bars);
                right -= 12.0;
            }
            let note = if e.connected {
                Some("Connected")
            } else if e.sae_only {
                Some("WPA3 only")
            } else if e.saved {
                Some(if e.bars.is_some() {
                    "Saved"
                } else {
                    "Saved, not nearby"
                })
            } else {
                None
            };
            if let Some(t) = note.and_then(|n| faces.get(NetFace::Small(n.into()))) {
                tex(out, t, right - t.1 as f32, mid - t.2 as f32 / 2.0);
            }
        }
    }

    fn draw_keyboard(
        &self,
        canvas: Canvas,
        target: &KeyTarget,
        faces: &NetFaces,
        out: &mut Vec<Draw>,
    ) {
        let kb = &self.keyboard;
        let prompt = match target {
            KeyTarget::Password { ssid } => format!("Password for {ssid}"),
            KeyTarget::HiddenName => "Name of the hidden network".to_string(),
            KeyTarget::HiddenPassword { ssid } => {
                format!("Password for {ssid} (leave empty if open)")
            }
            KeyTarget::AchUser => "Your RetroAchievements user name".to_string(),
            KeyTarget::AchPassword { user } => {
                format!("Password for {user} (only the login is kept)")
            }
        };
        let mut y = PLATE_H + 8.0;
        if let Some(t) = faces.get(NetFace::Small(prompt)) {
            tex(out, t, canvas.centred(t.1 as f32), y);
            y += t.2 as f32 + 4.0;
        } else {
            y += 26.0;
        }
        let field_w = canvas.w - 2.0 * 24.0;
        let field_h = 46.0;
        rect(out, 24.0, y, field_w, field_h, FIELD_FILL);
        let mut caret_x = 36.0;
        if !kb.text.is_empty() {
            if let Some(t) = faces.get(NetFace::Field(kb.text.clone())) {
                tex(out, t, 34.0, y + (field_h - t.2 as f32) / 2.0);
                caret_x = 34.0 + t.1 as f32;
            }
        }
        rect(out, caret_x, y + 10.0, 2.0, field_h - 20.0, KEY_LIT);
        y += field_h + 6.0;
        let note = kb.error.map(str::to_string).unwrap_or_else(|| {
            let n = kb.text.chars().count();
            match n {
                0 => String::new(),
                1 => "1 character".to_string(),
                n => format!("{n} characters"),
            }
        });
        if !note.is_empty() {
            if let Some(t) = faces.get(NetFace::Small(note)) {
                tex(out, t, canvas.centred(t.1 as f32), y);
            }
        }
        let keys_top = canvas.h - PLATE_H - 8.0 - 5.0 * KEY_H - 4.0 * KEY_GAP;
        for row in 0..5 {
            for col in 0..kb.row_len(row) {
                let b = key_box(kb, canvas, keys_top, row, col);
                let lit = row == kb.row && col == kb.col;
                rect(
                    out,
                    b.x,
                    b.y,
                    b.w,
                    b.h,
                    if lit { KEY_LIT } else { KEY_FILL },
                );
                let label = match kb.cap(row, col) {
                    Cap::Char(c) => c.to_string(),
                    Cap::Act(a) => a.text(kb.layer).to_string(),
                };
                if let Some(t) = faces.get(NetFace::Key(label, lit)) {
                    tex(
                        out,
                        t,
                        b.x + (b.w - t.1 as f32) / 2.0,
                        b.y + (b.h - t.2 as f32) / 2.0,
                    );
                }
            }
        }
    }

    pub fn hint_faces(
        &self,
        status: &NetStatus,
        faces: &NetFaces,
    ) -> (Vec<Hint>, Vec<TexId>, usize) {
        let (hints, ways_out) = self.hints(status);
        // Ask for every face before giving up on any, so one frame requests them all.
        let got: Vec<Option<TexId>> = hints
            .iter()
            .map(|h| {
                faces
                    .get(NetFace::Hint(h.key, h.label.clone()))
                    .map(|t| t.0)
            })
            .collect();
        let texes = got
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        (hints, texes, ways_out)
    }
}

/// A page of labelled rows laid out as the quick menu lays out its own: the pointer beside the
/// selected label, values on the right with arrows when they can change, and a window of
/// six rows that scrolls.
fn draw_rows(
    canvas: Canvas,
    rows: &[(&str, Shown)],
    selected: usize,
    faces: &NetFaces,
    out: &mut Vec<Draw>,
) {
    let n = rows.len();
    let selected = selected.min(n.saturating_sub(1));
    let shown = n.min(QUICK_VISIBLE);
    let start = selected
        .saturating_sub(shown / 2)
        .min(n.saturating_sub(shown));
    let block = QUICK_PITCH * shown as f32;
    let top = PLATE_H + (canvas.h - 2.0 * PLATE_H - block) / 2.0;
    let pad = MENU_PAD as f32;
    let right = canvas.w - QUICK_EDGE;
    for (slot, (i, (label, value))) in rows.iter().enumerate().skip(start).take(shown).enumerate() {
        let y = top + slot as f32 * QUICK_PITCH + TYPE_DROP;
        let lit = i == selected;
        let x = QUICK_LABEL_X - pad;
        if let Some(t) = faces.get(NetFace::Label((*label).into(), lit)) {
            tex(out, t, x, y);
        }
        if lit {
            if let Some(p) = faces.get(NetFace::Pointer) {
                tex(
                    out,
                    p,
                    x + pad - POINTER_GAP - p.1 as f32,
                    y - POINTER_BLEED,
                );
            }
        }
        let face = match value {
            Shown::Switch(t) => NetFace::Label(t.clone(), lit),
            Shown::Choice(t) | Shown::Text(t) => NetFace::Row(t.clone(), lit),
        };
        let arrows = lit && matches!(value, Shown::Switch(_) | Shown::Choice(_));
        // Ask for every face first, so one frame requests them all.
        let v = faces.get(face);
        let carets = match arrows {
            true => (
                faces.get(NetFace::Caret(false)),
                faces.get(NetFace::Caret(true)),
            ),
            false => (None, None),
        };
        let Some(v) = v else {
            continue;
        };
        let (Some(l), Some(r)) = carets else {
            tex(out, v, right + pad - v.1 as f32, y);
            continue;
        };
        let rx = right - r.1 as f32;
        tex(out, r, rx, y);
        let vx = rx - CARET_GAP + pad - v.1 as f32;
        tex(out, v, vx, y);
        tex(out, l, vx + pad - CARET_GAP - l.1 as f32, y);
    }
    if n > shown {
        let x = canvas.w - SCROLL_INSET - SCROLL_W;
        rect(out, x, top, SCROLL_W, block, [0.6, 0.6, 0.64, 0.3]);
        let thumb = block / n as f32;
        let along = selected as f32 / (n - 1) as f32;
        rect(
            out,
            x,
            top + (block - thumb) * along,
            SCROLL_W,
            thumb,
            BAR_ON,
        );
    }
}

fn draw_bars(out: &mut Vec<Draw>, x: f32, mid: f32, bars: u8) {
    for i in 0..4u8 {
        let h = 5.0 + 4.0 * i as f32;
        let colour = if i < bars.max(1) { BAR_ON } else { BAR_OFF };
        rect(out, x + i as f32 * 7.0, mid + 9.0 - h, 5.0, h, colour);
    }
}

fn split_id(id: &str) -> (String, String) {
    let groups: Vec<&str> = id.split('-').collect();
    let half = groups.len().div_ceil(2);
    (groups[..half].join("-"), groups[half..].join("-"))
}

/// The Tailscale login: a code to scan with a phone, and the link to type elsewhere.
fn draw_tail_login(canvas: Canvas, status: &NetStatus, faces: &NetFaces, out: &mut Vec<Draw>) {
    let url = match &status.tail {
        TailState::NeedsLogin { url } => url.clone(),
        _ => None,
    };
    let mut lines: Vec<NetFace> = Vec::new();
    let qr = url.as_ref().and_then(|u| faces.get(NetFace::Qr(u.clone())));
    let text_x = match qr {
        Some(q) => {
            let y = PLATE_H + (canvas.h - 2.0 * PLATE_H - q.2 as f32) / 2.0;
            tex(out, q, 24.0, y);
            24.0 + q.1 as f32 + 24.0
        }
        None => EDGE,
    };
    match (&status.tail, &url) {
        (TailState::Running { name, .. }, _) => {
            lines.push(NetFace::Value("Logged in".into(), true));
            if let Some(name) = name {
                lines.push(NetFace::Small(format!("This device is {name}")));
            }
        }
        (_, Some(url)) => {
            lines.push(NetFace::Value("Scan to log in".into(), true));
            lines.push(NetFace::Small("Or open this link on any".into()));
            lines.push(NetFace::Small("computer or phone:".into()));
            let (a, b) = split_url(url);
            lines.push(NetFace::Mono(a));
            if !b.is_empty() {
                lines.push(NetFace::Mono(b));
            }
            lines.push(NetFace::Small("This page moves on once".into()));
            lines.push(NetFace::Small("you have logged in.".into()));
        }
        (_, None) => {
            lines.push(NetFace::Value(tail_text(status), true));
            lines.push(NetFace::Small("Waiting for a login link…".into()));
        }
    }
    let mut y = PLATE_H + 16.0;
    for f in lines {
        let gap = matches!(f, NetFace::Small(_)) as u8 as f32 * 4.0;
        y += gap;
        if let Some(t) = faces.get(f) {
            tex(out, t, text_x, y);
            y += t.2 as f32 + 2.0;
        } else {
            y += 26.0;
        }
    }
}

/// Splits a long link after its host, so it fits beside the code.
fn split_url(url: &str) -> (String, String) {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    match rest.find('/') {
        Some(at) if url.len() > 30 => (rest[..at].to_string(), rest[at..].to_string()),
        _ => (rest.to_string(), String::new()),
    }
}

fn draw_sync(canvas: Canvas, status: &NetStatus, faces: &NetFaces, out: &mut Vec<Draw>) {
    let mut lines: Vec<NetFace> = Vec::new();
    let body_top = PLATE_H + 16.0;
    let qr = status
        .device_id
        .as_ref()
        .and_then(|id| faces.get(NetFace::Qr(id.clone())));
    let text_x = match qr {
        Some(q) => {
            let y = PLATE_H + (canvas.h - 2.0 * PLATE_H - q.2 as f32) / 2.0;
            tex(out, q, 24.0, y);
            24.0 + q.1 as f32 + 24.0
        }
        None => EDGE,
    };
    lines.push(NetFace::Value(sync_text(status), true));
    match (&status.sync, &status.device_id) {
        (SyncState::Off, _) => {
            lines.push(NetFace::Small(
                "Turn Syncthing on to get a device ID.".into(),
            ));
        }
        (SyncState::Missing, _) => {
            lines.push(NetFace::Small("Copy System/bin/syncthing from".into()));
            lines.push(NetFace::Small("the slot release onto this card.".into()));
        }
        (_, Some(id)) => {
            let (a, b) = split_id(id);
            lines.push(NetFace::Small("This device's ID".into()));
            lines.push(NetFace::Mono(a));
            lines.push(NetFace::Mono(b));
            if let Some(url) = status.web_url() {
                lines.push(NetFace::Small("Web interface".into()));
                lines.push(NetFace::Mono(url));
                lines.push(NetFace::Mono(format!(
                    "User {}   Password {}",
                    slot_net::syncthing::GUI_USER,
                    status.web_password
                )));
            }
            lines.push(NetFace::Small("Saves and States are offered as".into()));
            lines.push(NetFace::Small("folders slot-saves and slot-states.".into()));
        }
        (_, None) => {}
    }
    let mut y = body_top;
    for f in lines {
        let gap = matches!(f, NetFace::Small(_)) as u8 as f32 * 4.0;
        y += gap;
        if let Some(t) = faces.get(f) {
            tex(out, t, text_x, y);
            y += t.2 as f32 + 2.0;
        } else {
            y += 26.0;
        }
    }
}
