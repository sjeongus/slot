#![allow(clippy::field_reassign_with_default)]

mod common;

use std::time::{Duration, Instant};

use common::{app_booting_at, app_playing_in, tmp_root_with_carts, CLOCK_IS_SET};
use slot::app::{App, Phase};
use slot_input::{Action, Btn};
use slot_net::{NetConfig, NetStatus, SyncState, WifiState};
use slot_store::{write_slot_state, SlotState};
use slot_ui::{NetPage, QuickRow};
use tempfile::TempDir;

fn press(a: &mut App, btn: Btn) {
    a.apply(Action::GbaDown(btn));
    a.apply(Action::GbaUp(btn));
}

fn on_shelf_with_net() -> (TempDir, App) {
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    write_slot_state(
        d.path(),
        &SlotState {
            clock_set: true,
            ..SlotState::default()
        },
    )
    .unwrap();
    let (mut a, _) = app_booting_at(d.path(), CLOCK_IS_SET);
    assert!(matches!(a.phase(), Phase::Shelf), "{:?}", a.phase());
    a.set_net(slot_net::start(d.path(), None));
    (d, a)
}

fn open_network(a: &mut App) {
    a.apply(Action::QuickMenu);
    let at = QuickRow::MAIN
        .iter()
        .position(|r| *r == QuickRow::Network)
        .unwrap();
    for _ in 0..at {
        press(a, Btn::Down);
    }
    assert_eq!(a.quick_menu(), Some(QuickRow::Network));
    press(a, Btn::A);
    assert!(matches!(a.phase(), Phase::Network));
}

fn wait(a: &mut App, what: &str, ok: impl Fn(&NetStatus) -> bool) -> NetStatus {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        a.update(0.016);
        let s = a.net_status();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < until, "waited for {what}, last {s:?}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn the_network_row_sits_above_about() {
    let n = QuickRow::MAIN.len();
    assert_eq!(QuickRow::MAIN[n - 2], QuickRow::Network);
    assert_eq!(QuickRow::MAIN[n - 1], QuickRow::About);
}

#[test]
fn opening_without_a_network_thread_still_works_and_b_comes_back() {
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    write_slot_state(
        d.path(),
        &SlotState {
            clock_set: true,
            ..SlotState::default()
        },
    )
    .unwrap();
    let (mut a, _) = app_booting_at(d.path(), CLOCK_IS_SET);
    assert!(matches!(a.phase(), Phase::Shelf), "{:?}", a.phase());
    open_network(&mut a);
    let mut out = Vec::new();
    a.draw(&mut out);
    press(&mut a, Btn::B);
    assert_eq!(a.quick_menu(), Some(QuickRow::Network));
}

#[test]
fn choosing_a_network_and_typing_its_password_joins_and_saves_it() {
    let (d, mut a) = on_shelf_with_net();
    open_network(&mut a);
    wait(&mut a, "radio up", |s| s.wifi == WifiState::NoNetworks);
    // Network row, then the list.
    press(&mut a, Btn::Down);
    press(&mut a, Btn::A);
    assert_eq!(a.net_screen().page, NetPage::Networks);
    let s = wait(&mut a, "scan", |s| !s.scan.is_empty() && !s.scanning);
    assert_eq!(s.scan[0].ssid, "Home Wi-Fi");
    press(&mut a, Btn::A);
    assert!(matches!(a.net_screen().page, NetPage::Keyboard(_)));
    // "password1": the keyboard opens on q; walk to each letter.
    for c in "password1".chars() {
        let kb = &a.net_screen().keyboard;
        let (row, col) = (0..4)
            .find_map(|r| {
                (0..kb.row_len(r))
                    .find(|&c2| kb.cap(r, c2) == slot_ui::NetCap::Char(c))
                    .map(|c2| (r, c2))
            })
            .unwrap();
        while a.net_screen().keyboard.row != row {
            press(&mut a, Btn::Down);
        }
        while a.net_screen().keyboard.col != col {
            press(&mut a, Btn::Right);
        }
        press(&mut a, Btn::A);
    }
    assert_eq!(a.net_screen().keyboard.text, "password1");
    press(&mut a, Btn::Start);
    assert_eq!(a.net_screen().page, NetPage::Overview);
    let s = wait(&mut a, "connected", |s| {
        s.connected_ssid() == Some("Home Wi-Fi")
    });
    assert_eq!(s.ip(), Some("192.168.1.42"));
    assert_eq!(NetConfig::load(d.path()).networks[0].psk, "password1");
    let mut out = Vec::new();
    a.draw(&mut out);
    assert!(!out.is_empty());
}

#[test]
fn syncthing_pauses_while_a_game_runs() {
    let d = tmp_root_with_carts(&["Emerald"]);
    let mut c = NetConfig::default();
    c.syncthing = true;
    c.remember(slot_net::Network {
        ssid: "Coffee Shop".into(),
        psk: String::new(),
        hidden: false,
    });
    c.save(d.path()).unwrap();
    let mut a = app_playing_in(d.path(), "Emerald");
    a.set_net(slot_net::start(d.path(), None));
    wait(&mut a, "paused in game", |s| s.sync == SyncState::Paused);
}
