//! slot's own network features inside the App: the Syncthing and Tailscale pages under
//! General > Wi-Fi and the Achievements page under General, the info screens for a device ID or a login, keeping the network
//! thread in step with slot's Wi-Fi and with play, and the achievement notices over the game.
//!
//! Kept apart from app.rs so upstream changes to the App merge with as little friction as
//! possible: app.rs only routes these rows and the NetInfo phase here.

use std::collections::VecDeque;
use std::path::PathBuf;

use slot_input::{Action, Btn};
use slot_net::{Net, NetStatus, SyncState, TailState, WifiState};
use slot_ui::{
    ach_text, id_lines, info_width, sync_text, tail_text, AchView, Draw, InfoLine, InfoScreen,
    PageFaces, QuickRow, QuickValue,
};

use super::{App, KeyboardFor, LinkLegend, Phase};

/// The screen a row opens to show what does not fit in a row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NetInfo {
    /// Syncthing's device ID, its QR code, and the web interface login.
    SyncDevice,
    /// The code and link that log this device in to Tailscale.
    TailLogIn,
}

impl NetInfo {
    fn row(self) -> QuickRow {
        match self {
            NetInfo::SyncDevice => QuickRow::SyncDevice,
            NetInfo::TailLogIn => QuickRow::TailLogIn,
        }
    }
}

/// Everything slot's network features keep between frames.
#[derive(Default)]
pub struct NetUi {
    net: Option<Net>,
    status: Option<NetStatus>,
    seen: u64,
    pub faces: PageFaces,
    ach: AchView,
    ach_seen: u64,
    told_playing: Option<bool>,
    told_watching: bool,
    offset_at: Option<u64>,
    told_link: Option<Option<(String, Option<String>)>>,
    /// BaseOS says this model has no Wi-Fi (the RG28XX).
    no_wifi: bool,
    /// Achievement notices waiting to be shown, and when the first one went up.
    notices: VecDeque<Notice>,
    notice_at: Option<u64>,
}

impl NetUi {
    /// The rows this module answers for.
    pub fn owns(row: QuickRow) -> bool {
        matches!(row, net_rows!())
    }
}

/// A notice over the game: what happened, its name, a detail, and the badge if there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub heading: String,
    pub name: String,
    pub detail: Option<String>,
    pub badge: Option<PathBuf>,
}

/// How long each notice stays up, and how long it takes to slide in and out.
const NOTICE_MS: u64 = 4500;
const NOTICE_SLIDE_MS: u64 = 220;

fn notice_for(event: slot_ach::Event, on_page: bool) -> Option<Notice> {
    use slot_ach::Event;
    let n = |heading: &str, name: String, detail: Option<String>| Notice {
        heading: heading.to_string(),
        name,
        detail,
        badge: None,
    };
    Some(match event {
        Event::LoggedIn { .. } | Event::LoginFailed(_) if on_page => return None,
        Event::LoggedIn { user } => n("RetroAchievements", format!("Logged in as {user}"), None),
        Event::LoginFailed(e) => n("RetroAchievements", "Could not log in".into(), Some(e)),
        Event::GameLoaded {
            title,
            unlocked,
            total,
            badge,
            offline,
        } => Notice {
            badge,
            ..n(
                "Achievements",
                title,
                Some(format!(
                    "{unlocked} of {total} unlocked{}",
                    if offline { " · offline" } else { "" }
                )),
            )
        },
        Event::NoAchievements => n("Achievements", "None for this game".into(), None),
        Event::NotSaved => n(
            "Achievements",
            "Not saved for offline play yet".into(),
            Some("Saved the next time slot is online".into()),
        ),
        Event::GameFailed(e) => n("Achievements", e, None),
        Event::Unlocked {
            title,
            description,
            points,
            badge,
            later,
        } => Notice {
            badge,
            ..n(
                &format!(
                    "Unlocked · {points} point{}",
                    if points == 1 { "" } else { "s" }
                ),
                title,
                Some(match later {
                    true => "Saved, and sent once back online".into(),
                    false => description,
                }),
            )
        },
        Event::Mastered => n("Achievements", "Every achievement unlocked".into(), None),
        Event::Sent(1) => n("Achievements", "1 saved unlock sent".into(), None),
        Event::Sent(k) => n("Achievements", format!("{k} saved unlocks sent"), None),
        Event::ServerError(e) => {
            eprintln!("slot: achievements: {e}");
            return None;
        }
    })
}

fn ach_view(s: &slot_ach::AchStatus) -> AchView {
    use slot_ach::Account;
    AchView {
        enabled: s.enabled,
        user: s.user.clone(),
        logged_in: matches!(s.account, Account::LoggedIn { .. }),
        state: match &s.account {
            Account::LoggedOut => "Not logged in".into(),
            Account::LoggingIn => "Logging in".into(),
            Account::LoggedIn { offline: false } => "Logged in".into(),
            Account::LoggedIn { offline: true } => "Offline".into(),
            // The proxy's reasons are sentences; a row has room for a few words.
            Account::Failed(e) if e.starts_with("Offline") => "Log in once online".into(),
            Account::Failed(e) => e.clone(),
        },
        pending: s.pending,
        game: s
            .game
            .as_ref()
            .map(|g| format!("{} of {}", g.unlocked, g.total)),
        library: (s.library_total > 0).then(|| match s.library_saving {
            true => format!("Saving {} of {}", s.library_ready, s.library_total),
            false => format!("{} of {}", s.library_ready, s.library_total),
        }),
    }
}

/// What slot shows before the network thread has said anything.
fn quiet_status() -> NetStatus {
    NetStatus {
        wifi_on: false,
        wifi: WifiState::Off,
        scan: Vec::new(),
        scanning: false,
        saved: Vec::new(),
        syncthing_on: false,
        sync_in_game: false,
        sync: SyncState::Off,
        device_id: None,
        web_password: String::new(),
        network_time: false,
        time_synced: false,
        auto_timezone: false,
        timezone: None,
        retroarch_saves: slot_store::layout() == slot_store::Layout::RetroArch,
        online: false,
        tailscale_on: false,
        tail: TailState::Off,
        generation: 0,
    }
}

/// A long line cut into pieces that fit beside the QR code.
fn wrapped(text: &str, max_w: f32) -> Vec<String> {
    let per = ((max_w / 12.0) as usize).max(12);
    let chars: Vec<char> = text.chars().collect();
    chars.chunks(per).map(|c| c.iter().collect()).collect()
}

impl App {
    /// Hands the App the network thread. Without one, the pages still open and show
    /// everything off.
    pub fn set_net(&mut self, net: Net) {
        self.net.net = Some(net);
        self.net.seen = 0;
    }

    pub fn set_no_wifi(&mut self) {
        self.net.no_wifi = true;
    }

    pub fn net(&self) -> Option<&Net> {
        self.net.net.as_ref()
    }

    pub fn net_status(&self) -> NetStatus {
        self.net.status.clone().unwrap_or_else(quiet_status)
    }

    pub fn ach_view(&self) -> &AchView {
        &self.net.ach
    }

    pub fn net_faces(&self) -> &PageFaces {
        &self.net.faces
    }

    pub fn net_faces_mut(&mut self) -> &mut PageFaces {
        &mut self.net.faces
    }

    /// The switches among slot's own rows.
    pub(super) fn net_value(&self, row: QuickRow) -> Option<QuickValue> {
        let status = self.net.status.as_ref();
        Some(match row {
            QuickRow::SyncPower | QuickRow::Syncthing => {
                QuickValue::flag(status.is_some_and(|s| s.syncthing_on))
            }
            QuickRow::SyncInGame => QuickValue::flag(status.is_some_and(|s| s.sync_in_game)),
            QuickRow::TailPower | QuickRow::Tailscale => {
                QuickValue::flag(status.is_some_and(|s| s.tailscale_on))
            }
            QuickRow::AchPower | QuickRow::Achievements => QuickValue::flag(self.net.ach.enabled),
            _ => return None,
        })
    }

    /// The status text slot's own rows show on the right.
    pub fn net_texts(&self) -> Vec<(QuickRow, String)> {
        let page = match self.quick_menu() {
            Some(row) => row.page(),
            None => return Vec::new(),
        };
        let status = self.net_status();
        let ach = &self.net.ach;
        let mut texts = Vec::new();
        if page == QuickRow::SYNCTHING {
            texts.push((QuickRow::SyncStatus, sync_text(&status)));
        }
        if page == QuickRow::TAILSCALE {
            texts.push((QuickRow::TailStatus, tail_text(&status)));
            if let TailState::Running { ip, name, .. } = &status.tail {
                texts.push((QuickRow::TailAddress, ip.clone().unwrap_or_default()));
                texts.push((QuickRow::TailName, name.clone().unwrap_or_default()));
            }
        }
        if page == QuickRow::ACHIEVEMENTS {
            let account = match &ach.user {
                Some(user) if ach.logged_in => user.clone(),
                _ => "Log In".into(),
            };
            texts.push((QuickRow::AchAccount, account));
            texts.push((QuickRow::AchStatus, ach_text(ach)));
            texts.push((QuickRow::AchGame, ach.game.clone().unwrap_or_default()));
            texts.push((
                QuickRow::AchLibrary,
                ach.library.clone().unwrap_or_default(),
            ));
            let saved = match ach.pending {
                0 => "None".into(),
                n => format!("{n} to send"),
            };
            texts.push((QuickRow::AchSaved, saved));
        }
        texts
    }

    /// The rows that mean nothing in the current state.
    pub(super) fn net_hidden(&self) -> Vec<QuickRow> {
        let mut hidden = Vec::new();
        if self.net.no_wifi || self.net.net.is_none() {
            hidden.extend([QuickRow::Syncthing, QuickRow::Tailscale]);
        }
        let status = self.net_status();
        if !matches!(status.tail, TailState::Running { .. }) {
            hidden.extend([
                QuickRow::TailAddress,
                QuickRow::TailName,
                QuickRow::TailLogOut,
            ]);
        }
        if !matches!(status.tail, TailState::NeedsLogin { .. }) {
            hidden.push(QuickRow::TailLogIn);
        }
        let ach = &self.net.ach;
        if !ach.enabled {
            hidden.extend([
                QuickRow::AchAccount,
                QuickRow::AchStatus,
                QuickRow::AchGame,
                QuickRow::AchLibrary,
                QuickRow::AchSaved,
                QuickRow::AchLogOut,
            ]);
        }
        if ach.game.is_none() {
            hidden.push(QuickRow::AchGame);
        }
        if ach.library.is_none() {
            hidden.push(QuickRow::AchLibrary);
        }
        if !ach.logged_in {
            hidden.extend([QuickRow::AchLogOut, QuickRow::AchSaved]);
        }
        hidden
    }

    /// A on one of slot's own rows.
    pub(super) fn net_open(&mut self, row: QuickRow) {
        match row {
            QuickRow::Syncthing | QuickRow::Tailscale | QuickRow::Achievements => {
                if let Some(child) = row.child() {
                    let shown = child.shown(&self.hidden_rows());
                    if let Some(first) = shown.first() {
                        self.phase = Phase::QuickMenu { row: *first };
                    }
                }
            }
            QuickRow::SyncDevice => self.phase = Phase::NetInfo(NetInfo::SyncDevice),
            QuickRow::TailLogIn => self.phase = Phase::NetInfo(NetInfo::TailLogIn),
            QuickRow::TailLogOut => {
                if let Some(net) = &self.net.net {
                    net.tailscale_logout();
                }
                self.phase = Phase::QuickMenu {
                    row: QuickRow::TailPower,
                };
            }
            QuickRow::AchAccount => {
                let user = self.net.ach.user.clone().unwrap_or_default();
                self.open_keyboard(
                    "Username",
                    Some("RetroAchievements"),
                    KeyboardFor::AchUser,
                    slot_ui::Layout::General,
                    &user,
                );
            }
            QuickRow::AchSaved => slot_ach::send_now(),
            QuickRow::AchLogOut => {
                slot_ach::logout();
                self.phase = Phase::QuickMenu {
                    row: QuickRow::AchPower,
                };
            }
            _ => self.net_change(row),
        }
    }

    /// Left, right or A on one of slot's own switches.
    pub(super) fn net_change(&mut self, row: QuickRow) {
        let status = self.net_status();
        match row {
            QuickRow::AchPower => return slot_ach::set_enabled(!self.net.ach.enabled),
            QuickRow::SyncPower | QuickRow::SyncInGame | QuickRow::TailPower => {}
            _ => return,
        }
        let Some(net) = &self.net.net else {
            return;
        };
        match row {
            QuickRow::SyncPower => net.set_syncthing(!status.syncthing_on),
            QuickRow::SyncInGame => net.set_sync_in_game(!status.sync_in_game),
            QuickRow::TailPower => net.set_tailscale(!status.tailscale_on),
            _ => {}
        }
    }

    /// The username, then the password: only the token that comes back is kept.
    pub(super) fn ach_keyboard_done(&mut self, task: KeyboardFor, text: String) {
        match task {
            KeyboardFor::AchUser if text.trim().is_empty() => self.keyboard = None,
            KeyboardFor::AchUser => {
                let user = text.trim().to_string();
                self.open_keyboard(
                    "Password",
                    Some(&user.clone()),
                    KeyboardFor::AchPassword { user },
                    slot_ui::Layout::General,
                    "",
                );
            }
            KeyboardFor::AchPassword { user } => {
                self.keyboard = None;
                if !text.is_empty() {
                    slot_ach::login(&user, &text);
                }
            }
            _ => {}
        }
    }

    pub(super) fn net_info_input(&mut self, action: Action) {
        let Phase::NetInfo(info) = self.phase else {
            return;
        };
        if matches!(action, Action::GbaDown(Btn::B) | Action::QuickMenu) {
            self.phase = Phase::QuickMenu { row: info.row() };
        }
    }

    pub(super) fn draw_net_info(&self, info: NetInfo, out: &mut Vec<Draw>) {
        let canvas = self.canvas();
        let status = self.net_status();
        let max_w = info_width(canvas);
        let mut qr = None;
        let mut lines = Vec::new();
        match info {
            NetInfo::SyncDevice => match (&status.sync, &status.device_id) {
                (SyncState::Missing, _) => {
                    lines.push(InfoLine::value("Syncthing is not on this card"));
                    lines.push(InfoLine::label("Copy System/bin/syncthing"));
                    lines.push(InfoLine::label("from the slot release"));
                }
                (_, Some(id)) => {
                    qr = Some(id.clone());
                    lines.push(InfoLine::label("Device ID"));
                    lines.extend(id_lines(id).iter().map(|l| InfoLine::mono(l)));
                    if let Some(url) = status.web_url() {
                        lines.push(InfoLine::label("Web Interface"));
                        lines.push(InfoLine::mono(&url));
                        lines.push(InfoLine::mono(&format!(
                            "{} / {}",
                            slot_net::syncthing::GUI_USER,
                            status.web_password
                        )));
                    }
                    lines.push(InfoLine::label("Shares Saves and States"));
                }
                (_, None) => {
                    lines.push(InfoLine::value(&sync_text(&status)));
                    lines.push(InfoLine::label("Turn Syncthing on to get a device ID"));
                }
            },
            NetInfo::TailLogIn => match &status.tail {
                TailState::NeedsLogin { url: Some(url) } => {
                    qr = Some(url.clone());
                    lines.push(InfoLine::value("Scan to log in"));
                    lines.push(InfoLine::label("Or open this link"));
                    lines.extend(wrapped(url, max_w).iter().map(|l| InfoLine::mono(l)));
                    lines.push(InfoLine::label("Waiting for you to log in"));
                }
                TailState::Running { name, .. } => {
                    lines.push(InfoLine::value("Logged in"));
                    if let Some(name) = name {
                        lines.push(InfoLine::label(&format!("This device is {name}")));
                    }
                }
                _ => {
                    lines.push(InfoLine::value(&tail_text(&status)));
                    lines.push(InfoLine::label("Waiting for a login link"));
                }
            },
        }
        InfoScreen {
            qr,
            lines: &lines,
            ground: self.menu_ground(),
            canvas,
            faces: &self.net.faces,
        }
        .draw(out);
        super::draw_empty_slot(canvas, self.slot_faces, out);
        let back = self.link_faces(&[LinkLegend::Back]);
        self.draw_link_keys(&back, false, out);
    }

    /// Once a frame: picks up a new status, tells the network thread what slot's Wi-Fi is
    /// doing and whether a game is running, and turns achievement events into notices.
    pub(super) fn sync_net(&mut self) {
        // The RetroArch layout names states by file time; keep it in step with slot's clock.
        // Once a minute is plenty, and reading the clock is not free on every tick.
        let now = self.now();
        if self
            .net
            .offset_at
            .is_none_or(|at| now.saturating_sub(at) >= 60_000)
        {
            self.net.offset_at = Some(now);
            let utc = self.utc_secs();
            slot_store::set_wall_offset_min(self.state.zone.offset_min(utc) as i16);
        }
        self.sync_achievements();
        let snap = self.wifi();
        let link = match self.wifi_up() {
            true => Some((
                snap.status.ssid.clone().unwrap_or_default(),
                snap.status.ip.clone(),
            )),
            false => None,
        };
        if self.net.told_link.as_ref() != Some(&link) {
            slot_net::set_link(link.clone());
            self.net.told_link = Some(link);
        }
        let Some(net) = &self.net.net else {
            return;
        };
        let generation = net.generation();
        if generation != self.net.seen {
            let status = net.status();
            set_save_layout(status.retroarch_saves);
            self.net.status = Some(status);
            self.net.seen = generation;
        }
        // The login screen has done its job once the device is on the tailnet.
        if matches!(self.phase, Phase::NetInfo(NetInfo::TailLogIn))
            && matches!(
                self.net.status.as_ref().map(|s| &s.tail),
                Some(TailState::Running { .. })
            )
        {
            self.phase = Phase::QuickMenu {
                row: QuickRow::TailStatus,
            };
        }
        let Some(net) = &self.net.net else {
            return;
        };
        let playing = self.seated_cart().is_some();
        if self.net.told_playing != Some(playing) {
            net.playing(playing);
            self.net.told_playing = Some(playing);
        }
        // Someone is looking at a page: poll the services more often.
        let watching = matches!(self.phase, Phase::NetInfo(_))
            || self.quick_menu().is_some_and(|r| {
                r.page() == QuickRow::SYNCTHING || r.page() == QuickRow::TAILSCALE
            });
        if self.net.told_watching != watching {
            net.watching(watching);
            self.net.told_watching = watching;
        }
    }

    /// Something slot runs for itself needs the radio while a game is running.
    pub(super) fn net_wants_radio(&self) -> bool {
        if matches!(self.phase, Phase::Doze { .. }) || self.seated_cart().is_none() {
            return false;
        }
        let ach = &self.net.ach;
        let status = self.net.status.as_ref();
        (ach.enabled && ach.user.is_some())
            || status.is_some_and(|s| s.tailscale_on || (s.syncthing_on && s.sync_in_game))
    }

    pub(super) fn save_format_value(&self) -> QuickValue {
        match slot_store::layout() {
            slot_store::Layout::RetroArch => QuickValue::RetroArch,
            slot_store::Layout::Slot => QuickValue::SlotFormat,
        }
    }

    /// Settings > Gameplay > Save Format. Kept in Config/net.txt with the other fork settings:
    /// through the network thread when there is one, which owns that file, or directly.
    pub(super) fn toggle_save_format(&mut self) {
        let on = slot_store::layout() != slot_store::Layout::RetroArch;
        set_save_layout(on);
        if let Some(net) = &self.net.net {
            net.set_retroarch_saves(on);
        } else if let Some(root) = self.root() {
            let mut config = slot_net::NetConfig::load(root);
            config.retroarch_saves = on;
            if let Err(e) = config.save(root) {
                eprintln!("slot: net.txt: {e}");
            }
        }
    }

    /// Keeps the Achievements rows current and turns achievement events into notices.
    fn sync_achievements(&mut self) {
        let status = slot_ach::status();
        if status.generation != self.net.ach_seen || self.net.ach_seen == 0 {
            self.net.ach_seen = status.generation.max(1);
            self.net.ach = ach_view(&status);
        }
        let on_page = matches!(self.keyboard, Some((_, KeyboardFor::AchPassword { .. })))
            || self
                .quick_menu()
                .is_some_and(|r| r.page() == QuickRow::ACHIEVEMENTS);
        for event in slot_ach::take_events() {
            if let Some(n) = notice_for(event, on_page) {
                self.net.notices.push_back(n);
            }
        }
        let now = self.now();
        match self.net.notice_at {
            Some(at) if now.saturating_sub(at) >= NOTICE_MS => {
                self.net.notices.pop_front();
                self.net.notice_at = (!self.net.notices.is_empty()).then_some(now);
            }
            None if !self.net.notices.is_empty() => self.net.notice_at = Some(now),
            _ => {}
        }
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.net.notices.front()
    }

    /// The notice at the top of the screen, sliding in and out.
    pub(super) fn draw_notice(&self, out: &mut Vec<Draw>) {
        let (Some(n), Some(at)) = (self.net.notices.front(), self.net.notice_at) else {
            return;
        };
        let t = self.now().saturating_sub(at);
        let shown = match t {
            t if t < NOTICE_SLIDE_MS => t as f32 / NOTICE_SLIDE_MS as f32,
            t if t > NOTICE_MS - NOTICE_SLIDE_MS => {
                NOTICE_MS.saturating_sub(t) as f32 / NOTICE_SLIDE_MS as f32
            }
            _ => 1.0,
        };
        slot_ui::draw_notice(
            self.canvas(),
            &n.heading,
            &n.name,
            n.detail.as_deref(),
            n.badge.as_deref(),
            shown,
            &self.net.faces,
            out,
        );
    }

    /// Stops Syncthing and Tailscale before slot powers off, restarts or checks the card.
    pub(super) fn shutdown_net(&mut self) {
        if let Some(net) = self.net.net.take() {
            net.shutdown();
        }
    }
}

/// Switches every save and state from here on between RetroArch's layout and slot's own.
pub fn set_save_layout(retroarch: bool) {
    slot_store::set_layout(match retroarch {
        true => slot_store::Layout::RetroArch,
        false => slot_store::Layout::Slot,
    });
}
