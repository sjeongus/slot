//! The RetroArch layout. Every test here runs with it switched on, so this file is its own
//! test binary and never shares the process-wide setting with slot's own layout tests.

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use slot_store::{
    decode_retroarch, format_stamp, parse_stamp, retroarch_save_path, set_layout,
    set_wall_offset_min, Core, Layout, Platform, StateRing,
};

fn on() {
    set_layout(Layout::RetroArch);
    set_wall_offset_min(-420);
}

fn rzip(data: &[u8], codec: u8, chunk: usize) -> Vec<u8> {
    let mut out = b"#RZIPv".to_vec();
    out.push(codec);
    out.push(b'#');
    out.extend_from_slice(&(chunk as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    for piece in data.chunks(chunk) {
        let z = match codec {
            1 => miniz_oxide::deflate::compress_to_vec_zlib(piece, 6),
            _ => ruzstd::encoding::compress_to_vec(
                piece,
                ruzstd::encoding::CompressionLevel::Fastest,
            ),
        };
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
    }
    out
}

fn rastate(core: &[u8]) -> Vec<u8> {
    let block = |tag: &[u8; 4], body: &[u8]| {
        let mut b = tag.to_vec();
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(body);
        b.resize(8 + body.len().div_ceil(8) * 8, 0);
        b
    };
    let mut out = b"RASTATE\x01".to_vec();
    out.extend(block(b"RPLY", &[9u8; 13]));
    out.extend(block(b"MEM ", core));
    out.extend(block(b"ACHV", &[7u8; 5]));
    out.extend(block(b"END ", &[]));
    out
}

fn core_state() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn raw_states_pass_through() {
    let s = core_state();
    assert_eq!(decode_retroarch(s.clone()).unwrap(), s);
}

#[test]
fn rastate_gives_up_its_core_block() {
    let s = core_state();
    assert_eq!(decode_retroarch(rastate(&s)).unwrap(), s);
}

#[test]
fn rzip_deflate_and_zstd_both_inflate_across_chunks() {
    let s = core_state();
    for codec in [1u8, 2] {
        let packed = rzip(&rastate(&s), codec, 128 * 1024);
        assert!(packed.len() < s.len());
        assert_eq!(decode_retroarch(packed).unwrap(), s, "codec {codec}");
    }
}

#[test]
fn damaged_files_are_errors_not_garbage() {
    let s = core_state();
    let mut packed = rzip(&s, 1, 65536);
    packed.truncate(packed.len() - 10);
    assert!(decode_retroarch(packed).is_err());
    let mut bad = rastate(&s);
    bad.truncate(30);
    assert!(decode_retroarch(bad).is_err());
    let mut unknown = rzip(&s, 1, 65536);
    unknown[6] = 9;
    assert!(decode_retroarch(unknown).is_err());
}

#[test]
fn gba_saves_go_to_mgba_srm_and_gb_keeps_slots_layout() {
    on();
    let root = Path::new("/card");
    assert_eq!(
        retroarch_save_path(root, Platform::Gba, "Pokemon Unbound").unwrap(),
        Path::new("/card/Saves/mGBA/Pokemon Unbound.srm")
    );
    assert!(retroarch_save_path(root, Platform::Gb, "Tetris").is_none());
}

fn touch(path: &Path, bytes: &[u8], utc: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(utc))
        .unwrap();
}

const T0: u64 = 1_790_000_000;

#[test]
fn the_list_is_auto_then_highest_slot_down_and_has_no_cap() {
    on();
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state.auto"), b"auto", T0 + 5);
    touch(&dir.join("Emerald.state"), b"s0", T0 + 1);
    for n in 1..=14 {
        touch(&dir.join(format!("Emerald.state{n}")), b"sn", T0 + 10 + n);
    }
    touch(&dir.join("Emerald.state3.png"), b"png", T0);
    touch(&dir.join("Emeraldine.state"), b"other game", T0);
    touch(
        &dir.join("Emerald.state.auto.sync-conflict-1.png"),
        b"x",
        T0,
    );
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Mgba, "Emerald");
    let names: Vec<String> = ring
        .list()
        .unwrap()
        .iter()
        .map(|e| e.state.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut want = vec!["Emerald.state.auto".to_string()];
    want.extend((1..=14).rev().map(|n| format!("Emerald.state{n}")));
    want.push("Emerald.state".to_string());
    assert_eq!(names, want);
}

#[test]
fn stamps_are_local_wall_time_and_unique() {
    on();
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state"), b"a", T0);
    touch(&dir.join("Emerald.state1"), b"b", T0);
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Mgba, "Emerald");
    let stamps: Vec<i64> = ring
        .list()
        .unwrap()
        .iter()
        .map(|e| parse_stamp(&e.stamp).unwrap())
        .collect();
    let local = T0 as i64 - 420 * 60;
    assert!(stamps.contains(&local));
    assert!(stamps.contains(&(local + 1)), "{stamps:?}");
}

#[test]
fn a_new_save_goes_one_above_the_highest_and_keeps_its_stamp() {
    on();
    let d = tempfile::tempdir().unwrap();
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Mgba, "Emerald");
    let first = format_stamp(T0 as i64);
    ring.push(b"one", b"png1", &first).unwrap();
    assert!(d.path().join("States/mGBA/Emerald.state").exists());
    assert!(d.path().join("States/mGBA/Emerald.state.png").exists());
    let dir = d.path().join("States/mGBA");
    touch(&dir.join("Emerald.state7"), b"from retroarch", T0 + 100);
    let second = format_stamp(T0 as i64 + 200);
    ring.push(b"two", b"", &second).unwrap();
    assert!(dir.join("Emerald.state8").exists());
    let entries = ring.list().unwrap();
    assert_eq!(entries[0].state, dir.join("Emerald.state8"));
    assert_eq!(
        entries[0].stamp, second,
        "slot's own stamp survives FAT's 2 s rounding"
    );
    assert_eq!(
        ring.read(&first).unwrap(),
        (b"one".to_vec(), b"png1".to_vec())
    );
    ring.remove(&second).unwrap();
    assert!(!dir.join("Emerald.state8").exists());
}

#[test]
fn resume_is_the_newest_of_all_and_a_refused_one_moves_aside() {
    on();
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("States/mGBA");
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Mgba, "Emerald");
    assert_eq!(ring.read_resume().unwrap(), None);
    touch(&dir.join("Emerald.state.auto"), b"auto, older", T0);
    touch(
        &dir.join("Emerald.state4"),
        &rzip(&rastate(b"slot 4, newest"), 1, 4096),
        T0 + 60,
    );
    assert_eq!(ring.read_resume().unwrap().unwrap(), b"slot 4, newest");
    let moved = ring.retire_resume("2026-10-25_01-00-00").unwrap().unwrap();
    assert!(moved.starts_with(d.path().join("States/.slot/mGBA")));
    assert!(!dir.join("Emerald.state4").exists());
    assert_eq!(ring.read_resume().unwrap().unwrap(), b"auto, older");
    ring.write_resume(b"fresh").unwrap();
    assert_eq!(
        std::fs::read(dir.join("Emerald.state.auto")).unwrap(),
        b"fresh"
    );
}

#[test]
fn gpsp_states_get_retroarchs_gpsp_folder() {
    on();
    let d = tempfile::tempdir().unwrap();
    let ring = StateRing::new(d.path(), Platform::Gba, Core::Gpsp, "Emerald");
    ring.write_resume(b"x").unwrap();
    assert!(d.path().join("States/gpSP/Emerald.state.auto").exists());
}

#[test]
fn gb_states_keep_slots_ring() {
    on();
    let d = tempfile::tempdir().unwrap();
    let ring = StateRing::new(d.path(), Platform::Gb, Core::Mgba, "Tetris");
    ring.write_resume(b"x").unwrap();
    assert!(d.path().join("States/GB/mgba/Tetris/resume.state").exists());
}
