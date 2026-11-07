//! General > Achievements works with no network at all, and logs in through the
//! keyboard. In a file of its own: achievements are one per process.

mod common;

use std::path::Path;

use common::{app_booting_at, tmp_root_with_carts, CLOCK_IS_SET};
use slot::app::{App, KeyboardFor, Phase};
use slot_input::{Action, Btn};
use slot_store::{write_slot_state, SlotState};
use slot_ui::{QuickRow, QuickValue};

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

fn select(a: &mut App, row: QuickRow) {
    for _ in 0..16 {
        if a.quick_menu() == Some(row) {
            return;
        }
        press(a, Btn::Down);
    }
    panic!("no {row:?} on {:?}", a.phase());
}

fn open_achievements(a: &mut App) {
    a.apply(Action::QuickMenu);
    select(a, QuickRow::General);
    press(a, Btn::A);
    select(a, QuickRow::Achievements);
    press(a, Btn::A);
    assert_eq!(a.quick_menu(), Some(QuickRow::AchPower));
}

fn settle(a: &mut App) {
    for _ in 0..10 {
        a.update(0.016);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn achievements_stay_on_offline_and_the_account_row_asks_for_a_login() {
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
    for _ in 0..4 {
        let t = a.now();
        a.tick_ms(t + 1_000);
    }

    open_achievements(&mut a);
    assert_eq!(a.quick_value(QuickRow::AchPower), Some(QuickValue::On));
    for _ in 0..3 {
        press(&mut a, Btn::B);
    }
    assert!(matches!(a.phase(), Phase::Shelf), "{:?}", a.phase());
    open_achievements(&mut a);
    assert_eq!(
        a.quick_value(QuickRow::AchPower),
        Some(QuickValue::On),
        "reopened"
    );

    press(&mut a, Btn::Right);
    settle(&mut a);
    assert!(!slot_ach::status().enabled);
    assert_eq!(a.quick_value(QuickRow::AchPower), Some(QuickValue::Off));
    assert!(a.hidden_rows().contains(&QuickRow::AchAccount));
    press(&mut a, Btn::Right);
    settle(&mut a);
    assert!(slot_ach::status().enabled);

    select(&mut a, QuickRow::AchAccount);
    press(&mut a, Btn::A);
    assert_eq!(a.keyboard_for(), Some(&KeyboardFor::AchUser));
    assert_eq!(a.keyboard().map(|k| k.text()), Some("User".to_string()));
    press(&mut a, Btn::Start);
    assert_eq!(
        a.keyboard_for(),
        Some(&KeyboardFor::AchPassword {
            user: "User".into()
        })
    );
    press(&mut a, Btn::Y);
    assert!(a.keyboard().is_none());
    assert!(matches!(a.phase(), Phase::QuickMenu { .. }));
}
