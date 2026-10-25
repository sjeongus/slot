//! The Network screen inside the App, keeping the network thread in step with play, and the
//! achievement notices shown over the game.
//!
//! Kept apart from app.rs so upstream changes to the App merge with as little friction as
//! possible: app.rs only routes the Network phase here.

use std::collections::VecDeque;
use std::path::PathBuf;

use slot_input::{Action, Btn};
use slot_net::{Net, NetStatus, SyncState, TailState, WifiState};
use slot_ui::{
    draw_album_chrome, draw_menu_ground, AchView, AlbumChrome, Draw, NetFace, NetFaces, NetIntent,
    NetKey, NetPage, NetScreen, QuickRow,
};

use super::{App, Phase};

/// Everything the Network screen keeps between frames.
#[derive(Default)]
pub struct NetUi {
    net: Option<Net>,
    status: Option<NetStatus>,
    seen: u64,
    pub screen: NetScreen,
    pub faces: NetFaces,
    told_playing: Option<bool>,
    told_watching: bool,
    /// When the time zone's offset was last applied to slot's clock.
    zone_at: Option<u64>,
    /// BaseOS says this model has no Wi-Fi (the RG28XX), so the Network row is hidden.
    no_wifi: bool,
    ach_seen: u64,
    /// Achievement notices waiting to be shown, and when the first one went up.
    notices: VecDeque<Notice>,
    notice_at: Option<u64>,
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

fn notice_for(event: slot_ach::Event, on_network: bool) -> Option<Notice> {
    use slot_ach::Event;
    let n = |heading: &str, name: String, detail: Option<String>| Notice {
        heading: heading.to_string(),
        name,
        detail,
        badge: None,
    };
    Some(match event {
        Event::LoggedIn { .. } | Event::LoginFailed(_) if on_network => return None,
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
            Account::LoggingIn => "Logging in…".into(),
            Account::LoggedIn { offline: false } => "Logged in".into(),
            Account::LoggedIn { offline: true } => "Logged in, offline".into(),
            Account::Failed(e) => e.clone(),
        },
        pending: s.pending,
        game: s
            .game
            .as_ref()
            .map(|g| format!("{} of {} unlocked", g.unlocked, g.total)),
        library: (s.library_total > 0).then(|| match s.library_saving {
            true => format!("Saving {} of {}…", s.library_ready, s.library_total),
            false => format!("{} of {} saved", s.library_ready, s.library_total),
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
        tail: slot_net::TailState::Off,
        generation: 0,
    }
}

fn key_of(action: Action) -> Option<NetKey> {
    Some(match action {
        Action::GbaDown(Btn::Up) => NetKey::Up,
        Action::GbaDown(Btn::Down) => NetKey::Down,
        Action::GbaDown(Btn::Left) | Action::ShelfLeft => NetKey::Left,
        Action::GbaDown(Btn::Right) | Action::ShelfRight => NetKey::Right,
        Action::GbaDown(Btn::A) => NetKey::A,
        Action::GbaDown(Btn::B) => NetKey::B,
        Action::GbaDown(Btn::X) => NetKey::X,
        Action::GbaDown(Btn::Y) => NetKey::Y,
        Action::GbaDown(Btn::Start) => NetKey::Start,
        Action::GbaDown(Btn::Select) => NetKey::Select,
        Action::QuickMenu => NetKey::Menu,
        _ => return None,
    })
}

impl App {
    /// Hands the App the network thread. Without one, the screen still opens and shows
    /// everything off.
    pub fn set_net(&mut self, net: Net) {
        self.net.net = Some(net);
        self.net.seen = 0;
    }

    pub fn set_no_wifi(&mut self) {
        self.net.no_wifi = true;
    }

    pub fn no_wifi(&self) -> bool {
        self.net.no_wifi
    }

    pub fn net(&self) -> Option<&Net> {
        self.net.net.as_ref()
    }

    pub fn net_status(&self) -> NetStatus {
        self.net.status.clone().unwrap_or_else(quiet_status)
    }

    pub fn net_screen(&self) -> &NetScreen {
        &self.net.screen
    }

    pub fn net_faces(&self) -> &NetFaces {
        &self.net.faces
    }

    pub fn net_faces_mut(&mut self) -> &mut NetFaces {
        &mut self.net.faces
    }

    pub(super) fn open_network(&mut self) {
        self.net.screen = NetScreen::new();
        self.phase = Phase::Network;
        self.sync_net();
    }

    pub(super) fn network_input(&mut self, action: Action) {
        let Some(key) = key_of(action) else {
            return;
        };
        let status = self.net_status();
        let intent = self.net.screen.input(key, &status);
        // Achievements are slot's own, with or without the network thread.
        match &intent {
            NetIntent::SetAchievements(on) => return slot_ach::set_enabled(*on),
            NetIntent::AchLogin { user, password } => return slot_ach::login(user, password),
            NetIntent::AchLogout => return slot_ach::logout(),
            NetIntent::AchSend => return slot_ach::send_now(),
            _ => {}
        }
        let Some(net) = self.net.net.clone() else {
            if intent == NetIntent::Close {
                self.close_network();
            }
            return;
        };
        match intent {
            NetIntent::None => {}
            NetIntent::Close => self.close_network(),
            NetIntent::SetWifi(on) => {
                net.set_wifi(on);
                if on && self.net.screen.page == NetPage::Networks {
                    net.scan();
                }
            }
            NetIntent::Scan => net.scan(),
            NetIntent::Join { ssid, psk, hidden } => net.join(&ssid, &psk, hidden),
            NetIntent::JoinSaved(ssid) => net.join_saved(&ssid),
            NetIntent::Forget(ssid) => net.forget(&ssid),
            NetIntent::SetSyncthing(on) => net.set_syncthing(on),
            NetIntent::SetSyncInGame(on) => net.set_sync_in_game(on),
            NetIntent::SetNetworkTime(on) => net.set_network_time(on),
            NetIntent::SetAutoTimezone(on) => {
                net.set_auto_timezone(on);
                self.net.zone_at = None;
            }
            NetIntent::SetTailscale(on) => net.set_tailscale(on),
            NetIntent::TailLogout => net.tailscale_logout(),
            NetIntent::SetAchievements(_)
            | NetIntent::AchLogin { .. }
            | NetIntent::AchLogout
            | NetIntent::AchSend => {}
        }
    }

    fn close_network(&mut self) {
        self.phase = Phase::QuickMenu {
            row: QuickRow::Network,
        };
        self.sync_net();
    }

    /// Once a frame: picks up a new status, and tells the network thread whether a game is
    /// running and whether anyone is looking at the screen.
    pub(super) fn sync_net(&mut self) {
        // The RetroArch layout names states by file time; keep it in step with slot's clock.
        slot_store::set_wall_offset_min(self.state.utc_offset_min);
        self.sync_achievements();
        let Some(net) = &self.net.net else {
            return;
        };
        let generation = net.generation();
        if generation != self.net.seen {
            let status = net.status();
            set_save_layout(status.retroarch_saves);
            slot_ui::set_online(status.online);
            self.net.status = Some(status);
            self.net.seen = generation;
            self.net.zone_at = None;
        }
        self.follow_zone();
        // The login page has done its job once the device is on the tailnet.
        if self.net.screen.page == NetPage::TailLogin
            && matches!(
                self.net.status.as_ref().map(|s| &s.tail),
                Some(TailState::Running { .. })
            )
        {
            self.net.screen.page = NetPage::Tailscale;
        }
        let Some(net) = &self.net.net else {
            return;
        };
        let playing = matches!(
            self.phase,
            Phase::Inserting { .. } | Phase::Playing { .. } | Phase::Ejecting { .. }
        ) || matches!(self.phase, Phase::Doze { cart: Some(_) });
        if self.net.told_playing != Some(playing) {
            net.playing(playing);
            self.net.told_playing = Some(playing);
        }
        let watching = matches!(self.phase, Phase::Network);
        if self.net.told_watching != watching {
            net.watching(watching);
            self.net.told_watching = watching;
        }
    }

    /// With the time zone on automatic, keeps slot's UTC offset at the zone's current one, so
    /// the clock moves for daylight saving by itself. Checked every 30 s, which is plenty.
    fn follow_zone(&mut self) {
        let Some(status) = &self.net.status else {
            return;
        };
        let (true, Some(zone)) = (status.auto_timezone, status.timezone.clone()) else {
            return;
        };
        let now = self.now();
        if self
            .net
            .zone_at
            .is_some_and(|at| now.saturating_sub(at) < 30_000)
        {
            return;
        }
        self.net.zone_at = Some(now);
        let Some(secs) = slot_net::time::zone_offset_secs(&zone, self.utc_secs()) else {
            return;
        };
        let minutes = (secs / 60).clamp(
            i32::from(slot_store::UTC_OFFSET_MIN),
            i32::from(slot_store::UTC_OFFSET_MAX),
        ) as i16;
        if minutes != self.state.utc_offset_min {
            eprintln!("slot: clock: {zone} is UTC{minutes:+} min now");
            self.state.utc_offset_min = minutes;
            self.persist();
        }
    }

    pub(super) fn save_format_value(&self) -> slot_ui::QuickValue {
        match slot_store::layout() {
            slot_store::Layout::RetroArch => slot_ui::QuickValue::RetroArch,
            slot_store::Layout::Slot => slot_ui::QuickValue::SlotFormat,
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

    /// Keeps the Achievements page's summary current and turns achievement events into
    /// notices.
    fn sync_achievements(&mut self) {
        let status = slot_ach::status();
        if status.generation != self.net.ach_seen {
            self.net.ach_seen = status.generation;
            self.net.screen.ach = ach_view(&status);
        }
        let on_network = matches!(self.phase, Phase::Network);
        for event in slot_ach::take_events() {
            if let Some(n) = notice_for(event, on_network) {
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

    /// Stops Syncthing and Wi-Fi before slot powers off, restarts or checks the card.
    pub(super) fn shutdown_net(&mut self) {
        if let Some(net) = self.net.net.take() {
            net.shutdown();
        }
    }

    pub(super) fn draw_network(&self, out: &mut Vec<Draw>) {
        let canvas = self.canvas();
        let status = self.net_status();
        draw_menu_ground(canvas, self.menu_ground(), out);
        let faces = &self.net.faces;
        self.net.screen.draw(canvas, &status, faces, out);
        let title = faces
            .get(NetFace::Title(self.net.screen.title().to_string()))
            .map(|t| t.0);
        let (hints, hint_faces, ways_out) = self.net.screen.hint_faces(&status, faces);
        draw_album_chrome(
            canvas,
            AlbumChrome {
                title,
                hints: &hints,
                hint_faces: &hint_faces,
                ways_out,
                dots: None,
            },
            self.battery,
            self.battery_percent,
            self.hud_bolt,
            self.shelf_clock,
            out,
        );
    }
}

/// Switches every save and state from here on between RetroArch's layout and slot's own.
pub fn set_save_layout(retroarch: bool) {
    slot_store::set_layout(match retroarch {
        true => slot_store::Layout::RetroArch,
        false => slot_store::Layout::Slot,
    });
}
