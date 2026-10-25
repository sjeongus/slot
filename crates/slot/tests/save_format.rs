//! Settings > Gameplay > Save Format. In a file of its own: the save layout is global to the
//! process, and tests elsewhere pick their own.

mod common;

use common::{app_booting_at, app_playing_in, tmp_root_with_carts, CLOCK_IS_SET};
use slot::app::App;
use slot::app::Phase;
use slot_input::{Action, Btn};
use slot_net::NetConfig;
use slot_store::{layout, write_slot_state, Layout, SlotState};
use slot_ui::{QuickRow, QuickValue};

fn press(a: &mut App, btn: Btn) {
    a.apply(Action::GbaDown(btn));
    a.apply(Action::GbaUp(btn));
}

#[test]
fn save_format_sits_at_the_end_of_gameplay_and_is_kept_in_net_txt() {
    assert_eq!(QuickRow::GAME.last(), Some(&QuickRow::SaveFormat));
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
    slot::app::set_save_layout(true);
    assert_eq!(
        a.quick_value(QuickRow::SaveFormat),
        Some(QuickValue::RetroArch)
    );

    a.apply(Action::QuickMenu);
    for _ in 0..QuickRow::Game.position(&a.hidden_rows()) {
        press(&mut a, Btn::Down);
    }
    assert_eq!(a.quick_menu(), Some(QuickRow::Game));
    press(&mut a, Btn::A);
    press(&mut a, Btn::Up);
    assert_eq!(
        a.quick_menu(),
        Some(QuickRow::SaveFormat),
        "up from the top wraps to Save Format"
    );
    press(&mut a, Btn::Right);
    assert_eq!(layout(), Layout::Slot);
    assert_eq!(
        a.quick_value(QuickRow::SaveFormat),
        Some(QuickValue::SlotFormat)
    );
    assert!(!NetConfig::load(d.path()).retroarch_saves);

    press(&mut a, Btn::Left);
    assert_eq!(layout(), Layout::RetroArch);
    assert!(NetConfig::load(d.path()).retroarch_saves);

    drop(a);
    let playing = app_playing_in(d.path(), "Emerald");
    assert!(
        playing.in_game_hidden().contains(&QuickRow::SaveFormat),
        "not offered under a running game"
    );
}
