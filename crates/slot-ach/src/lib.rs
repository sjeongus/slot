//! RetroAchievements for slot.
//!
//! [rcheevos](https://github.com/RetroAchievements/rcheevos) watches the game's memory and
//! decides when an achievement is earned. slot answers its server requests itself
//! ([`proxy`]): online they go to RetroAchievements through BaseOS's `curl`, and what comes
//! back is kept on the card; offline, slot answers from the card and queues unlocks until it
//! is back online, in the manner of RAOfflineProxy. Unlocks are softcore only:
//! RetroAchievements grants hardcore only to emulators it has approved.
//!
//! The emulator thread drives a game through a [`Session`]; the menus read [`status`] and
//! drain [`take_events`] for the notices shown over the game.

mod client;
pub mod proxy;
pub mod settings;
pub mod store;
pub mod transport;

use std::path::{Path, PathBuf};

pub use settings::Settings;
use slot_retro::RetroCore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Account {
    LoggedOut,
    LoggingIn,
    /// `offline`: logged in from what the card remembered.
    LoggedIn {
        offline: bool,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameProgress {
    pub id: u32,
    pub title: String,
    pub hash: String,
    pub badge: String,
    pub unlocked: u32,
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AchStatus {
    pub enabled: bool,
    pub user: Option<String>,
    pub account: Account,
    /// Unlocks earned offline, waiting to be sent.
    pub pending: usize,
    /// The game being played, once its achievements have loaded.
    pub game: Option<GameProgress>,
    /// ROMs on the card, and how many of them are saved for offline play.
    pub library_total: usize,
    pub library_ready: usize,
    /// Saving them now.
    pub library_saving: bool,
    pub generation: u64,
}

impl Default for AchStatus {
    fn default() -> Self {
        AchStatus {
            enabled: false,
            user: None,
            account: Account::LoggedOut,
            pending: 0,
            game: None,
            library_total: 0,
            library_ready: 0,
            library_saving: false,
            generation: 0,
        }
    }
}

/// Things worth a notice on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    LoggedIn {
        user: String,
    },
    LoginFailed(String),
    GameLoaded {
        title: String,
        unlocked: u32,
        total: u32,
        badge: Option<PathBuf>,
        offline: bool,
    },
    /// RetroAchievements has no achievements for this ROM.
    NoAchievements,
    /// Offline, and this game's achievements were not saved while online.
    NotSaved,
    GameFailed(String),
    Unlocked {
        title: String,
        description: String,
        points: u32,
        badge: Option<PathBuf>,
        /// Earned offline: saved, and sent once slot is back online.
        later: bool,
    },
    Mastered,
    /// Unlocks earned offline that have now been sent.
    Sent(usize),
    ServerError(String),
}

/// Starts achievements for the card at `root`: logs in with the saved account if they are on.
/// `user_agent` names slot to RetroAchievements.
pub fn start(root: &Path, user_agent: &str) {
    client::start(
        root,
        Box::new(transport::Curl::new(user_agent)),
        slot_net::online,
    );
}

/// Starts with another way to reach the server and to tell whether slot is online.
pub fn start_with(root: &Path, transport: Box<dyn transport::Transport>, online: fn() -> bool) {
    client::start(root, transport, online);
}

pub fn status() -> AchStatus {
    client::core().map(|c| c.status()).unwrap_or_default()
}

pub fn take_events() -> Vec<Event> {
    client::core().map(|c| c.take_events()).unwrap_or_default()
}

pub fn set_enabled(on: bool) {
    if let Some(c) = client::core() {
        c.set_enabled(on);
    }
}

/// Logs in with a password. Only the token that comes back is kept.
pub fn login(user: &str, password: &str) {
    if let Some(c) = client::core() {
        c.login(user, password);
    }
}

pub fn logout() {
    if let Some(c) = client::core() {
        c.logout();
    }
}

/// The card's ROMs. While online and logged in, slot saves each one's achievements for
/// offline play in the background, a few at a time and never during a game.
pub fn set_library(roms: Vec<PathBuf>) {
    if let Some(c) = client::core() {
        c.set_library(roms);
    }
}

/// Tries to send unlocks earned offline now, rather than at the next half-minute check.
pub fn send_now() {
    if let Some(c) = client::core() {
        c.send_now();
    }
}

/// RetroAchievements' console for a ROM, from its extension.
pub fn console_for(rom: &Path) -> Option<u32> {
    let ext = rom.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "gb" => Some(4),
        "gba" => Some(5),
        "gbc" => Some(6),
        _ => None,
    }
}

/// Where the running core keeps its memory.
pub struct Memory {
    descs: Vec<client::Desc>,
    system: (*mut u8, usize),
    save: (*mut u8, usize),
}

impl Memory {
    pub fn of(core: &mut dyn RetroCore) -> Memory {
        let descs = core
            .memory_map()
            .into_iter()
            .map(|d| client::Desc {
                flags: d.flags,
                ptr: d.ptr.cast(),
                offset: d.offset,
                start: d.start,
                select: d.select,
                disconnect: d.disconnect,
                len: d.len,
            })
            .collect();
        let null = (std::ptr::null_mut(), 0);
        Memory {
            descs,
            system: core
                .memory_raw(slot_retro::MEMORY_SYSTEM_RAM)
                .unwrap_or(null),
            save: core.memory_raw(slot_retro::MEMORY_SAVE_RAM).unwrap_or(null),
        }
    }
}

/// Achievements for one game, on the emulator thread. Dropping it unloads the game; do that
/// before the core goes away.
pub struct Session {
    _private: (),
}

impl Session {
    /// Starts watching `rom` if achievements are on and an account is set up.
    pub fn begin(rom: &Path, core: &mut dyn RetroCore) -> Option<Session> {
        let c = client::core()?;
        let console = console_for(rom)?;
        let memory = Memory::of(core);
        c.begin_game(console, rom, &memory)
            .then_some(Session { _private: () })
    }

    /// After every frame the core runs.
    pub fn frame(&mut self) {
        if let Some(c) = client::core() {
            c.frame();
        }
    }

    /// While the game is paused, now and then.
    pub fn idle(&mut self) {
        if let Some(c) = client::core() {
            c.idle();
        }
    }

    /// After a state is loaded or the game rewinds: what was primed no longer holds.
    pub fn reset(&mut self) {
        if let Some(c) = client::core() {
            c.reset();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(c) = client::core() {
            c.end_game();
        }
    }
}
