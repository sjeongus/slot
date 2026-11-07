//! slot's own pages under General > Wi-Fi: Syncthing and Tailscale follow slot's Wi-Fi, and
//! the pages read as the rest of the quick menu does.
#![allow(clippy::field_reassign_with_default)]

mod common;

use std::time::{Duration, Instant};

use common::{app_booting_at, tmp_root_with_carts, CLOCK_IS_SET};
use slot::app::{App, Phase};
use slot::wifi::{Fake, Wifi};
use slot_input::{Action, Btn};
use slot_net::{NetConfig, NetStatus, SyncState, WifiState};
use slot_store::{write_slot_state, SlotState};
use slot_ui::{QuickRow, QuickValue};
use tempfile::TempDir;

/// What slot's Wi-Fi is doing is one per process, as on the device: one app at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn press(a: &mut App, btn: Btn) {
    a.apply(Action::GbaDown(btn));
    a.apply(Action::GbaUp(btn));
}

fn settle(a: &mut App) {
    for _ in 0..4 {
        let t = a.now();
        a.tick_ms(t + 1_000);
    }
}

fn on_shelf(config: NetConfig, wifi: Option<Fake>) -> (TempDir, App) {
    slot_net::set_link(None);
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    write_slot_state(
        d.path(),
        &SlotState {
            clock_set: true,
            ..SlotState::default()
        },
    )
    .unwrap();
    config.save(d.path()).unwrap();
    let (mut a, _) = app_booting_at(d.path(), CLOCK_IS_SET);
    assert!(matches!(a.phase(), Phase::Shelf), "{:?}", a.phase());
    if let Some(fake) = wifi {
        std::fs::write(d.path().join("Config/wifi.conf"), b"network={}\n").unwrap();
        a.set_wifi(Wifi::inline(Box::new(fake)));
    }
    a.set_net(slot_net::start(d.path(), None));
    settle(&mut a);
    (d, a)
}

fn select(a: &mut App, row: QuickRow) {
    for _ in 0..16 {
        if a.quick_menu() == Some(row) {
            return;
        }
        press(a, Btn::Down);
    }
    panic!("no {row:?} on {:?}", a.phase());
}

fn open_wifi(a: &mut App) {
    a.apply(Action::QuickMenu);
    select(a, QuickRow::General);
    press(a, Btn::A);
    select(a, QuickRow::Wifi);
    press(a, Btn::A);
    assert_eq!(a.quick_menu(), Some(QuickRow::WifiPower));
}

fn wait(a: &mut App, what: &str, ok: impl Fn(&NetStatus) -> bool) -> NetStatus {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        settle(a);
        let s = a.net_status();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < until, "waited for {what}, last {s:?}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn syncthing_tailscale_and_achievements_sit_under_wifi() {
    assert_eq!(
        QuickRow::WIFI.map(QuickRow::label),
        ["Wi-Fi", "Network", "Achievements", "Syncthing", "Tailscale"]
    );
    for (page, parent) in [
        (&QuickRow::SYNCTHING[..], &QuickRow::WIFI[..]),
        (&QuickRow::TAILSCALE, &QuickRow::WIFI),
        (&QuickRow::ACHIEVEMENTS, &QuickRow::WIFI),
    ] {
        assert_eq!(page[0].parent().map(|p| p.page()), Some(parent));
    }
}

#[test]
fn the_pages_open_with_no_network() {
    let _g = serial();
    let (_d, mut a) = on_shelf(NetConfig::default(), None);
    open_wifi(&mut a);
    select(&mut a, QuickRow::Tailscale);
    press(&mut a, Btn::A);
    assert_eq!(a.quick_menu(), Some(QuickRow::TailPower));
    assert!(a
        .net_texts()
        .iter()
        .any(|(r, t)| *r == QuickRow::TailStatus && t == "Off"));
}

#[test]
fn syncthing_starts_once_slots_wifi_has_joined_and_switches_from_its_page() {
    let _g = serial();
    let mut config = NetConfig::default();
    config.syncthing = true;
    let (d, mut a) = on_shelf(config, Some(Fake::joined()));
    let s = wait(
        &mut a,
        "joined",
        |s| matches!(&s.wifi, WifiState::Connected { ssid, .. } if ssid == "Home"),
    );
    assert!(s.web_url().is_some(), "{s:?}");
    wait(&mut a, "running", |s| {
        matches!(s.sync, SyncState::Running { .. })
    });

    open_wifi(&mut a);
    select(&mut a, QuickRow::Syncthing);
    press(&mut a, Btn::A);
    assert_eq!(a.quick_menu(), Some(QuickRow::SyncPower));
    assert_eq!(a.quick_value(QuickRow::SyncPower), Some(QuickValue::On));
    assert!(a
        .net_texts()
        .iter()
        .any(|(r, t)| *r == QuickRow::SyncStatus && !t.is_empty()));
    press(&mut a, Btn::Right);
    wait(&mut a, "off", |s| {
        !s.syncthing_on && s.sync == SyncState::Off
    });
    assert!(!NetConfig::load(d.path()).syncthing);

    select(&mut a, QuickRow::SyncDevice);
    press(&mut a, Btn::A);
    assert!(matches!(a.phase(), Phase::NetInfo(_)), "{:?}", a.phase());
    let mut out = Vec::new();
    a.draw(&mut out);
    press(&mut a, Btn::B);
    assert_eq!(a.quick_menu(), Some(QuickRow::SyncDevice));
}

#[test]
fn a_model_without_wifi_hides_syncthing_and_tailscale() {
    let _g = serial();
    let (_d, mut a) = on_shelf(NetConfig::default(), None);
    assert!(!a.hidden_rows().contains(&QuickRow::Syncthing));
    a.set_no_wifi();
    let hidden = a.hidden_rows();
    assert!(hidden.contains(&QuickRow::Syncthing) && hidden.contains(&QuickRow::Tailscale));
    assert!(!hidden.contains(&QuickRow::Achievements));
}

#[test]
fn the_old_wifi_and_clock_settings_are_handed_to_slot() {
    let _g = serial();
    let mut config = NetConfig::default();
    config.wifi = false;
    config.network_time = true;
    config.auto_timezone = true;
    let (d, _a) = on_shelf(config, None);
    let c = NetConfig::load(d.path());
    assert!(c.wifi && !c.network_time && !c.auto_timezone);
}
