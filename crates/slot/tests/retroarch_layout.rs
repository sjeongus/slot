//! Playing under the RetroArch layout: what lands on the card, and what loads. This binary
//! switches the process-wide layout on, so it never shares a process with slot's own tests.

mod common;

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use common::{app_playing_in, app_playing_with, tmp_root_with_carts, StubSnapshot};
use slot::app::{set_save_layout, Phase};
use slot::persist;
use slot_input::Action;
use slot_store::{Core, Platform, StateRing};

fn touch(path: &Path, bytes: &[u8], utc: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(utc))
        .unwrap();
}

#[test]
fn saving_a_state_writes_retroarchs_next_slot_with_a_thumbnail() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald"]);
    let dir = d.path().join("States/mGBA");
    touch(
        &dir.join("Emerald.state11"),
        b"from retroarch",
        1_700_000_000,
    );
    let mut a = app_playing_in(d.path(), "Emerald");
    a.apply_at(Action::SaveState, 1_000);
    assert_eq!(
        std::fs::read(dir.join("Emerald.state12")).unwrap(),
        vec![9u8; 1024]
    );
    assert_eq!(
        std::fs::read(dir.join("Emerald.state12.png")).unwrap(),
        b"png"
    );
    assert!(
        !d.path()
            .join("States/GBA")
            .join("mgba")
            .join("Emerald")
            .exists(),
        "nothing went to slot's own layout"
    );
}

#[test]
fn ejecting_writes_state_auto() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    let mut a = app_playing_in(d.path(), "Emerald");
    a.apply(Action::Eject);
    assert!(matches!(a.phase(), Phase::Ejecting { .. }));
    assert_eq!(
        std::fs::read(d.path().join("States/mGBA/Emerald.state.auto")).unwrap(),
        vec![9u8; 1024]
    );
}

#[test]
fn battery_saves_round_trip_through_mgba_srm_and_old_ones_still_read() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald"]);
    let old = d.path().join("Saves/GBA/Emerald.sav");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, [3u8; 32]).unwrap();
    assert_eq!(
        persist::read_sav(d.path(), Platform::Gba, "Emerald").unwrap(),
        Some(vec![3u8; 32]),
        "slot's old save is still found until a new one is written"
    );
    assert!(persist::write_sav(d.path(), Platform::Gba, "Emerald", &[3u8; 32]).unwrap());
    assert_eq!(
        std::fs::read(d.path().join("Saves/mGBA/Emerald.srm")).unwrap(),
        vec![3u8; 32]
    );
    assert!(old.exists(), "the old file is left alone");
    assert!(!persist::write_sav(d.path(), Platform::Gba, "Emerald", &[3u8; 32]).unwrap());
    std::fs::write(d.path().join("Saves/mGBA/Emerald.srm"), [4u8; 32]).unwrap();
    assert_eq!(
        persist::read_sav(d.path(), Platform::Gba, "Emerald").unwrap(),
        Some(vec![4u8; 32]),
        "RetroArch's .srm wins"
    );
}

#[test]
fn load_state_takes_the_newest_by_time_not_the_top_of_the_list() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald"]);
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state.auto"), &[1u8; 16], 1_700_000_000);
    touch(&dir.join("Emerald.state3"), &[3u8; 16], 1_700_000_500);
    touch(&dir.join("Emerald.state5"), &[5u8; 16], 1_700_000_100);
    let (snapshot, loaded) = StubSnapshot::pair();
    let mut a = app_playing_with(d.path(), "Emerald", snapshot);
    a.apply(Action::LoadState);
    assert_eq!(loaded.lock().unwrap().as_deref(), Some(&[3u8; 16][..]));
}

#[test]
fn the_switcher_lists_auto_then_the_highest_slot_down() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald"]);
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state.auto"), &[1u8; 16], 1_700_000_000);
    touch(&dir.join("Emerald.state"), &[0u8; 16], 1_700_000_900);
    for n in [3u32, 10, 2] {
        touch(
            &dir.join(format!("Emerald.state{n}")),
            &[n as u8; 16],
            1_700_000_100,
        );
    }
    let mut a = app_playing_in(d.path(), "Emerald");
    a.apply(Action::Polaroids);
    let names: Vec<String> = a
        .polaroid_entries()
        .iter()
        .map(|e| e.state.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "Emerald.state.auto",
            "Emerald.state10",
            "Emerald.state3",
            "Emerald.state2",
            "Emerald.state"
        ]
    );
}

#[test]
fn inserting_resumes_from_the_newest_state_of_all() {
    set_save_layout(true);
    let d = tmp_root_with_carts(&["Emerald"]);
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state.auto"), &[1u8; 16], 1_700_000_000);
    touch(&dir.join("Emerald.state7"), &[7u8; 16], 1_700_009_000);
    let resume = persist::read_resume(d.path(), Platform::Gba, Core::Mgba, "Emerald")
        .unwrap()
        .unwrap();
    assert_eq!(resume, vec![7u8; 16]);
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Mgba, "Emerald");
    assert_eq!(ring.list().unwrap().len(), 2);
}
