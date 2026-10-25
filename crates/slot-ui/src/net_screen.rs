//! Settings > Network: Wi-Fi on and off, choosing a network, typing its password with the
//! D-pad, and Syncthing.
//!
//! The screen is plain state. slot hands it buttons and the latest [`NetStatus`]; it answers
//! with a [`NetIntent`] for slot to pass on to the network thread, and draws itself from text
//! faces that the frontend rasterises on demand (see [`NetFace`]).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use slot_net::{valid_passphrase, NetStatus, ScanResult, SyncState, WifiState};

use crate::draw::{Canvas, Draw, TexId};
use crate::hud::PLATE_H;
use crate::plate::{hint_coloured, title_face, Hint, UndoFace};
use crate::power_menu::MENU_INK;
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
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum OverviewRow {
    Wifi,
    Network,
    Address,
    Syncthing,
    SyncInGame,
    Details,
}

impl OverviewRow {
    pub const ALL: [OverviewRow; 6] = [
        OverviewRow::Wifi,
        OverviewRow::Network,
        OverviewRow::Address,
        OverviewRow::Syncthing,
        OverviewRow::SyncInGame,
        OverviewRow::Details,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OverviewRow::Wifi => "Wi-Fi",
            OverviewRow::Network => "Network",
            OverviewRow::Address => "IP Address",
            OverviewRow::Syncthing => "Syncthing",
            OverviewRow::SyncInGame => "Sync In Games",
            OverviewRow::Details => "Syncthing Details",
        }
    }

    fn toggles(self) -> bool {
        matches!(
            self,
            OverviewRow::Wifi | OverviewRow::Syncthing | OverviewRow::SyncInGame
        )
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyTarget {
    /// The password for a network seen in a scan.
    Password { ssid: String },
    /// The name of a network that does not broadcast it.
    HiddenName,
    /// Then its password. Left empty for an open network.
    HiddenPassword { ssid: String },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NetPage {
    Overview,
    Networks,
    Keyboard(KeyTarget),
    Sync,
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
    pub pick: usize,
    pub keyboard: Keyboard,
}

impl Default for NetScreen {
    fn default() -> Self {
        Self::new()
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

pub fn wifi_text(status: &NetStatus) -> String {
    match &status.wifi {
        WifiState::Off => "Off".into(),
        WifiState::Linking => "In use by link".into(),
        WifiState::NoNetworks => "Choose a network".into(),
        WifiState::Searching => "Searching…".into(),
        WifiState::Joining { ssid } => format!("Joining {ssid}…"),
        WifiState::Connected { ssid, .. } => ssid.clone(),
        WifiState::Failed { ssid, reason } if ssid.is_empty() => reason.clone(),
        WifiState::Failed { ssid, reason } => format!("{ssid}: {reason}"),
    }
}

pub fn sync_text(status: &NetStatus) -> String {
    match &status.sync {
        SyncState::Off => "Off".into(),
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
            pick: 0,
            keyboard: Keyboard::new(),
        }
    }

    pub fn overview_row(&self) -> OverviewRow {
        OverviewRow::ALL[self.row.min(OverviewRow::ALL.len() - 1)]
    }

    pub fn input(&mut self, key: NetKey, status: &NetStatus) -> NetIntent {
        match self.page.clone() {
            NetPage::Overview => self.overview_input(key, status),
            NetPage::Networks => self.networks_input(key, status),
            NetPage::Keyboard(target) => self.keyboard_input(key, target),
            NetPage::Sync => match key {
                NetKey::B | NetKey::Menu => {
                    self.page = NetPage::Overview;
                    NetIntent::None
                }
                _ => NetIntent::None,
            },
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
            OverviewRow::Syncthing => NetIntent::SetSyncthing(!status.syncthing_on),
            OverviewRow::SyncInGame => NetIntent::SetSyncInGame(!status.sync_in_game),
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
                OverviewRow::Details => {
                    self.page = NetPage::Sync;
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
                self.page = NetPage::Networks;
                NetIntent::None
            }
            NetKey::Start => self.finish(target),
        }
    }

    fn finish(&mut self, target: KeyTarget) -> NetIntent {
        let text = self.keyboard.text.clone();
        match target {
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
            NetPage::Keyboard(_) => "Password",
            NetPage::Sync => "Syncthing",
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
                    OverviewRow::Network | OverviewRow::Details => Some("Open"),
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
            NetPage::Sync => (vec![h("B", "Back")], 1),
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
    /// Row labels, in the quick menu's capitals.
    Label(String, bool),
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
}

const DIM: [u8; 3] = [0x9a, 0x9a, 0xa4];
const DARK: [u8; 3] = [0x1a, 0x1a, 0x1e];
const LABEL_PX: f32 = 26.0;
const VALUE_PX: f32 = 22.0;
const SMALL_PX: f32 = 17.0;
const MONO_PX: f32 = 19.0;
const KEY_PX: f32 = 22.0;
const FIELD_PX: f32 = 26.0;
const MAX_FACE_W: u32 = 600;
const QR_SCALE: u32 = 4;

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

/// A label in the quick menu's style: capitals, letter spaced.
fn label_face(label: &str, ink: [u8; 3]) -> UndoFace {
    let Some(font) = text::label_font() else {
        return empty();
    };
    let layout = text::fit(font, label, MAX_FACE_W as f32, 1, LABEL_PX, 16.0);
    let w = layout
        .lines
        .iter()
        .map(|l| text::line_width(font, l, layout.px, layout.tracking))
        .fold(0.0, f32::max)
        .ceil() as u32
        + 2;
    let h = 36;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    text::draw_centred(&mut rgba, w, h, &layout, ink);
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
    let side = (n + 2 * quiet) * QR_SCALE;
    let mut rgba = vec![0xffu8; (side * side * 4) as usize];
    let colors = code.to_colors();
    for my in 0..n {
        for mx in 0..n {
            if colors[(my * n + mx) as usize] != qrcode::Color::Dark {
                continue;
            }
            for py in 0..QR_SCALE {
                for px in 0..QR_SCALE {
                    let x = (mx + quiet) * QR_SCALE + px;
                    let y = (my + quiet) * QR_SCALE + py;
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
        NetFace::Label(t, lit) => label_face(t, ink(*lit)),
        NetFace::Value(t, lit) => plain_face(t, VALUE_PX, ink(*lit), 340),
        NetFace::Small(t) => plain_face(t, SMALL_PX, DIM, MAX_FACE_W),
        NetFace::Mono(t) => plain_face(t, MONO_PX, MENU_INK, MAX_FACE_W),
        NetFace::Key(t, lit) => plain_face(t, KEY_PX, if *lit { DARK } else { MENU_INK }, 160),
        NetFace::Field(t) => plain_face(t, FIELD_PX, MENU_INK, 560),
        NetFace::Hint(key, label) => hint_coloured(key, label, MENU_INK),
        NetFace::Lock(lit) => lock_face(*lit),
        NetFace::Qr(data) => qr_face(data),
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
const ROW_PITCH: f32 = 50.0;
const LIST_PITCH: f32 = 46.0;
const LIST_SHOWN: usize = 7;
const HIGHLIGHT: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
const KEY_FILL: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
const KEY_LIT: [f32; 4] = [0.965, 0.957, 0.937, 1.0];
const FIELD_FILL: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const BAR_ON: [f32; 4] = [0.965, 0.957, 0.937, 1.0];
const BAR_OFF: [f32; 4] = [1.0, 1.0, 1.0, 0.18];
const KEY_GAP: f32 = 7.0;
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
            NetPage::Overview => self.draw_overview(canvas, status, faces, out),
            NetPage::Networks => self.draw_networks(canvas, status, faces, out),
            NetPage::Keyboard(target) => self.draw_keyboard(canvas, target, faces, out),
            NetPage::Sync => draw_sync(canvas, status, faces, out),
        }
    }

    fn draw_overview(
        &self,
        canvas: Canvas,
        status: &NetStatus,
        faces: &NetFaces,
        out: &mut Vec<Draw>,
    ) {
        let rows = OverviewRow::ALL.len() as f32;
        let top = PLATE_H + (canvas.h - 2.0 * PLATE_H - rows * ROW_PITCH) / 2.0;
        for (i, row) in OverviewRow::ALL.iter().enumerate() {
            let y = top + i as f32 * ROW_PITCH;
            let lit = i == self.row;
            if lit {
                rect(
                    out,
                    EDGE / 2.0,
                    y,
                    canvas.w - EDGE,
                    ROW_PITCH - 4.0,
                    HIGHLIGHT,
                );
            }
            if let Some(t) = faces.get(NetFace::Label(row.label().into(), lit)) {
                tex(out, t, EDGE, y + (ROW_PITCH - 4.0 - t.2 as f32) / 2.0);
            }
            let value = match row {
                OverviewRow::Wifi => on_off(status.wifi_on).to_string(),
                OverviewRow::Network => wifi_text(status),
                OverviewRow::Address => status.ip().unwrap_or("—").to_string(),
                OverviewRow::Syncthing => on_off(status.syncthing_on).to_string(),
                OverviewRow::SyncInGame => on_off(status.sync_in_game).to_string(),
                OverviewRow::Details => sync_text(status),
            };
            if let Some(t) = faces.get(NetFace::Value(value, lit)) {
                tex(
                    out,
                    t,
                    canvas.w - EDGE - t.1 as f32,
                    y + (ROW_PITCH - 4.0 - t.2 as f32) / 2.0,
                );
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
