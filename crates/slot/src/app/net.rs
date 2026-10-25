//! The Network screen inside the App, and keeping the network thread in step with play.
//!
//! Kept apart from app.rs so upstream changes to the App merge with as little friction as
//! possible: app.rs only routes the Network phase here.

use slot_input::{Action, Btn};
use slot_net::{Net, NetStatus, SyncState, WifiState};
use slot_ui::{
    draw_album_chrome, draw_menu_ground, AlbumChrome, Draw, NetFace, NetFaces, NetIntent, NetKey,
    NetPage, NetScreen, QuickRow,
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
    /// BaseOS says this model has no Wi-Fi (the RG28XX), so the Network row is hidden.
    no_wifi: bool,
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
        let Some(net) = &self.net.net else {
            return;
        };
        let generation = net.generation();
        if generation != self.net.seen {
            self.net.status = Some(net.status());
            self.net.seen = generation;
        }
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
