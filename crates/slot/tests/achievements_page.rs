//! The Network page shows achievements as they are, however often it is opened and whatever
//! Wi-Fi is doing. In a file of its own: achievements are one per process.

mod common;

use std::path::Path;

use common::{app_booting_at, tmp_root_with_carts, CLOCK_IS_SET};
use slot::app::{App, Phase};
use slot_input::{Action, Btn};
use slot_store::{write_slot_state, SlotState};
use slot_ui::QuickRow;

struct NoNetwork;

impl slot_ach::transport::Transport for NoNetwork {
    fn post(&self, _url: &str, _body: &str, _ct: &str) -> slot_ach::transport::Reply {
        slot_ach::transport::Reply::Unreachable
    }
    fn fetch(&self, _url: &str, _dest: &Path) -> bool {
        false
    }
}

fn press(a: &mut App, btn: Btn) {
    a.apply(Action::GbaDown(btn));
    a.apply(Action::GbaUp(btn));
}

fn open_network(a: &mut App) {
    if a.quick_menu() != Some(QuickRow::Network) {
        a.apply(Action::QuickMenu);
        for _ in 0..QuickRow::Network.position(&a.hidden_rows()) {
            press(a, Btn::Down);
        }
    }
    press(a, Btn::A);
    assert!(matches!(a.phase(), Phase::Network));
}

#[test]
fn achievements_read_on_after_wifi_goes_off_and_the_page_reopens() {
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    write_slot_state(
        d.path(),
        &SlotState {
            clock_set: true,
            ..SlotState::default()
        },
    )
    .unwrap();
    slot_ach::Settings {
        enabled: true,
        user: "User".into(),
        token: "ApiToken".into(),
    }
    .save(d.path())
    .unwrap();
    slot_ach::start_with(d.path(), Box::new(NoNetwork), || false);
    let (mut a, _) = app_booting_at(d.path(), CLOCK_IS_SET);
    a.set_net(slot_net::start(d.path(), None));
    a.update(0.016);

    open_network(&mut a);
    assert!(a.net_screen().ach.enabled, "on when first opened");
    press(&mut a, Btn::A); // Wi-Fi is the first row: off.
    for _ in 0..20 {
        a.update(0.016);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!a.net_status().wifi_on);
    press(&mut a, Btn::B);
    open_network(&mut a);
    assert!(a.net_screen().ach.enabled, "still on after Wi-Fi went off");
    assert!(slot_ach::status().enabled);
}
