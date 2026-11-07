//! Renders this fork's own screens the way the device draws them, and dumps them as PNGs
//! when SCRATCH_PNG_DIR is set: Wi-Fi with its new rows, Syncthing, Tailscale and Achievements, their
//! info screens, the keyboard and Save Format. One test, because achievements are one per
//! process. Skipped where there is no GL to draw with.

mod common;

use std::path::Path;

use common::{clocked, tmp_root_with_carts};
use slot::frontend::Frontend;
use slot::wifi::{Fake, Wifi};
use slot_gfx::{Compositor, HeadlessSurface, OUT_H, OUT_W};
use slot_input::{Action, Btn};
use slot_power::SimPlatform;
use slot_ui::QuickRow;

struct Offline;

impl slot_ach::transport::Transport for Offline {
    fn post(&self, _url: &str, _body: &str, _ct: &str) -> slot_ach::transport::Reply {
        slot_ach::transport::Reply::Unreachable
    }
    fn fetch(&self, _url: &str, _dest: &Path) -> bool {
        false
    }
}

fn settle(f: &mut Frontend) {
    for _ in 0..4 {
        let t = f.app().now();
        f.app_mut().tick_ms(t + 1_000);
    }
}

fn press(f: &mut Frontend, btn: Btn) {
    f.app_mut().apply(Action::GbaDown(btn));
    f.app_mut().apply(Action::GbaUp(btn));
}

fn select(f: &mut Frontend, row: QuickRow) {
    for _ in 0..16 {
        if f.app().quick_menu() == Some(row) {
            return;
        }
        press(f, Btn::Down);
    }
    panic!("no {row:?} on {:?}", f.app().phase());
}

fn wait(f: &mut Frontend, what: &str, ok: impl Fn(&slot_net::NetStatus) -> bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !ok(&f.app().net_status()) {
        settle(f);
        assert!(std::time::Instant::now() < until, "waited for {what}");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// Text faces are built on the frame after a draw asks for them: draw a few times.
fn shot(f: &mut Frontend, c: &mut Compositor, name: &str) -> Vec<u8> {
    for _ in 0..4 {
        f.compose(c);
    }
    let frame = c.read_frame();
    if let Ok(dir) = std::env::var("SCRATCH_PNG_DIR") {
        let file = std::fs::File::create(format!("{dir}/fork-{name}.png")).unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(file), OUT_W, OUT_H);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&frame).unwrap();
    }
    frame
}

fn lit(frame: &[u8]) -> usize {
    frame
        .chunks(4)
        .filter(|p| p[..3].iter().all(|c| *c > 0xc0))
        .count()
}

#[test]
fn the_forks_screens_render() {
    let Ok(surface) = HeadlessSurface::new() else {
        return;
    };
    let Ok(mut c) = Compositor::new(&surface) else {
        return;
    };
    let d = tmp_root_with_carts(&["Emerald", "Fusion"]);
    clocked(d.path());
    std::fs::write(d.path().join("Config/wifi.conf"), b"network={}\n").unwrap();
    let mut config = slot_net::NetConfig::default();
    config.syncthing = true;
    config.tailscale = true;
    config.save(d.path()).unwrap();
    slot_ach::Settings {
        enabled: true,
        user: "sjeongus".into(),
        token: "ApiToken".into(),
    }
    .save(d.path())
    .unwrap();
    // A login saved while online, so the page shows the account offline.
    slot_ach::store::Store::new(d.path()).save_login(
        "sjeongus",
        r#"{"Success":true,"User":"sjeongus","Token":"ApiToken","Score":1,"SoftcoreScore":1,"Messages":0,"Permissions":1,"AccountType":"Registered"}"#,
    );
    slot_ach::start_with(d.path(), Box::new(Offline), || false);
    let mut f = Frontend::boot(Box::new(SimPlatform::at(d.path().to_path_buf())));
    f.upload_faces(&mut c);
    f.app_mut().set_wifi(Wifi::inline(Box::new(Fake::joined())));
    f.app_mut().set_net(slot_net::start(d.path(), None));
    settle(&mut f);
    wait(&mut f, "Syncthing", |s| {
        matches!(s.sync, slot_net::SyncState::Running { .. })
    });

    f.app_mut().apply(Action::QuickMenu);
    select(&mut f, QuickRow::General);
    press(&mut f, Btn::A);
    select(&mut f, QuickRow::Wifi);
    press(&mut f, Btn::A);
    select(&mut f, QuickRow::Syncthing);
    let wifi = shot(&mut f, &mut c, "wifi");
    assert!(lit(&wifi) > 500);

    press(&mut f, Btn::A);
    shot(&mut f, &mut c, "syncthing");
    select(&mut f, QuickRow::SyncDevice);
    press(&mut f, Btn::A);
    let info = shot(&mut f, &mut c, "syncthing-device");
    assert!(lit(&info) > 20_000, "the QR code is drawn");
    press(&mut f, Btn::B);
    press(&mut f, Btn::B);

    select(&mut f, QuickRow::Tailscale);
    press(&mut f, Btn::A);
    wait(&mut f, "a login link", |s| {
        matches!(s.tail, slot_net::TailState::NeedsLogin { url: Some(_) })
    });
    shot(&mut f, &mut c, "tailscale-login-row");
    select(&mut f, QuickRow::TailLogIn);
    press(&mut f, Btn::A);
    shot(&mut f, &mut c, "tailscale-login");
    wait(&mut f, "logged in", |s| {
        matches!(s.tail, slot_net::TailState::Running { .. })
    });
    settle(&mut f);
    assert_eq!(f.app().quick_menu(), Some(QuickRow::TailStatus));
    shot(&mut f, &mut c, "tailscale");
    press(&mut f, Btn::B);

    select(&mut f, QuickRow::Achievements);
    press(&mut f, Btn::A);
    settle(&mut f);
    shot(&mut f, &mut c, "achievements");
    select(&mut f, QuickRow::AchAccount);
    press(&mut f, Btn::A);
    shot(&mut f, &mut c, "achievements-keyboard");
    press(&mut f, Btn::R2);
    shot(&mut f, &mut c, "keyboard-symbols");
    f.app_mut().apply(Action::QuickMenu);
    shot(&mut f, &mut c, "keyboard-help");
    press(&mut f, Btn::B);
    press(&mut f, Btn::Y);

    for _ in 0..3 {
        press(&mut f, Btn::B);
    }
    assert_eq!(f.app().quick_menu(), Some(QuickRow::General));
    select(&mut f, QuickRow::Game);
    press(&mut f, Btn::A);
    press(&mut f, Btn::Up);
    assert_eq!(f.app().quick_menu(), Some(QuickRow::SaveFormat));
    shot(&mut f, &mut c, "gameplay-save-format");
}
