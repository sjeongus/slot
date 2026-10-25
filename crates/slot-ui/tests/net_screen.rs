use slot_gfx::{Canvas, Draw, TexId};
use slot_net::{NetStatus, ScanResult, SyncState, WifiState};
use slot_ui::{
    net_entries, net_face, qr_face, NetCap, NetFace, NetFaces, NetIntent, NetKey, NetKeyboard,
    NetLayer, NetPage, NetScreen, OverviewRow, SyncRow, UndoFace, OTHER_NETWORK,
};

fn scan(ssid: &str, dbm: i32, secured: bool) -> ScanResult {
    ScanResult {
        ssid: ssid.into(),
        bssid: "02:00:00:00:00:01".into(),
        freq: 2437,
        signal_dbm: dbm,
        secured,
        sae_only: false,
    }
}

fn status() -> NetStatus {
    NetStatus {
        wifi_on: true,
        wifi: WifiState::Connected {
            ssid: "Home Wi-Fi".into(),
            ip: Some("192.168.1.42".into()),
        },
        scan: vec![
            scan("Coffee Shop", -71, false),
            scan("Home Wi-Fi", -48, true),
            scan("Neighbour 5G", -79, true),
        ],
        scanning: false,
        saved: vec!["Home Wi-Fi".into(), "Grandma's".into()],
        syncthing_on: true,
        sync_in_game: false,
        sync: SyncState::Running {
            devices: 2,
            busy: false,
        },
        device_id: Some("MFZWI3D-BONSGYC-YLTMRWG-C43ENR5-QXGZDMM-FZWI3DP-BONSGYY-LTMRWAD".into()),
        web_password: "k4pz9qw2ma".into(),
        network_time: true,
        time_synced: true,
        auto_timezone: true,
        timezone: Some("America/Los_Angeles".into()),
        retroarch_saves: true,
        online: true,
        tailscale_on: true,
        tail: slot_net::TailState::Running {
            ip: Some("100.101.102.103".into()),
            name: Some("slot-rg35xxh".into()),
            tailnet: Some("me@example.com".into()),
        },
        generation: 1,
    }
}

#[test]
fn the_list_puts_the_connected_network_first_and_ends_with_other() {
    let e = net_entries(&status());
    let names: Vec<&str> = e.iter().map(|e| e.ssid.as_str()).collect();
    assert_eq!(
        names,
        [
            "Home Wi-Fi",
            "Coffee Shop",
            "Neighbour 5G",
            "Grandma's",
            OTHER_NETWORK
        ]
    );
    assert!(e[0].connected && e[0].saved);
    assert_eq!(e[3].bars, None, "a saved network out of range has no bars");
    assert!(e[4].other);
}

#[test]
fn overview_toggles_ask_for_the_opposite_state() {
    let s = status();
    let mut n = NetScreen::new();
    assert_eq!(n.overview_row(), OverviewRow::Wifi);
    assert_eq!(n.input(NetKey::A, &s), NetIntent::SetWifi(false));
    assert_eq!(n.input(NetKey::Right, &s), NetIntent::SetWifi(false));
    for _ in 0..6 {
        n.input(NetKey::Down, &s);
    }
    assert_eq!(n.overview_row(), OverviewRow::NetworkTime);
    assert_eq!(n.input(NetKey::A, &s), NetIntent::SetNetworkTime(false));
    n.input(NetKey::Down, &s);
    assert_eq!(
        n.input(NetKey::Right, &s),
        NetIntent::SetAutoTimezone(false)
    );
    n.input(NetKey::Down, &s);
    assert_eq!(n.overview_row(), OverviewRow::Wifi, "the list wraps");
    assert_eq!(n.input(NetKey::B, &s), NetIntent::Close);
}

#[test]
fn syncthing_has_a_page_of_its_own() {
    let s = status();
    let mut n = NetScreen::new();
    for _ in 0..3 {
        n.input(NetKey::Down, &s);
    }
    assert_eq!(n.overview_row(), OverviewRow::Syncthing);
    assert_eq!(
        n.input(NetKey::Right, &s),
        NetIntent::None,
        "it opens, not toggles"
    );
    assert_eq!(n.input(NetKey::A, &s), NetIntent::None);
    assert_eq!(n.page, NetPage::Syncthing);
    assert_eq!(n.title(), "Syncthing");
    assert_eq!(n.sync_row(), SyncRow::Syncthing);
    assert_eq!(n.input(NetKey::A, &s), NetIntent::SetSyncthing(false));
    n.input(NetKey::Down, &s);
    assert_eq!(n.input(NetKey::Left, &s), NetIntent::SetSyncInGame(true));
    n.input(NetKey::Down, &s);
    assert_eq!(n.sync_row(), SyncRow::Details);
    n.input(NetKey::A, &s);
    assert_eq!(n.page, NetPage::Sync);
    n.input(NetKey::B, &s);
    assert_eq!(
        n.page,
        NetPage::Syncthing,
        "back from the details is the Syncthing page"
    );
    n.input(NetKey::B, &s);
    assert_eq!(n.page, NetPage::Overview);
    assert_eq!(
        n.overview_row(),
        OverviewRow::Syncthing,
        "and back again lands on its row"
    );
}

#[test]
fn switches_read_in_capitals_and_names_keep_their_case() {
    let mut s = status();
    assert_eq!(slot_ui::time_text(&s), "ON, synced");
    s.time_synced = false;
    assert_eq!(slot_ui::time_text(&s), "ON");
    s.network_time = false;
    assert_eq!(slot_ui::time_text(&s), "OFF");
    s.wifi_on = false;
    s.wifi = slot_net::WifiState::Off;
    assert_eq!(slot_ui::wifi_text(&s), "OFF");
    s.wifi = slot_net::WifiState::Connected {
        ssid: "Home Wi-Fi".into(),
        ip: Some("192.168.1.20".into()),
    };
    assert_eq!(slot_ui::wifi_text(&s), "Home Wi-Fi");
    s.sync = slot_net::SyncState::Off;
    assert_eq!(slot_ui::sync_text(&s), "OFF");
}

#[test]
fn opening_networks_scans_and_a_secured_one_asks_for_its_password() {
    let s = status();
    let mut n = NetScreen::new();
    n.input(NetKey::Down, &s);
    assert_eq!(n.input(NetKey::A, &s), NetIntent::Scan);
    assert_eq!(n.page, NetPage::Networks);
    // Home is connected: A does nothing.
    assert_eq!(n.input(NetKey::A, &s), NetIntent::None);
    n.input(NetKey::Down, &s);
    // Coffee Shop is open: joins straight away.
    assert_eq!(
        n.input(NetKey::A, &s),
        NetIntent::Join {
            ssid: "Coffee Shop".into(),
            psk: String::new(),
            hidden: false
        }
    );
    assert_eq!(n.page, NetPage::Overview);
    assert_eq!(n.overview_row(), OverviewRow::Network);
    n.input(NetKey::A, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    assert!(matches!(n.page, NetPage::Keyboard(_)));
}

#[test]
fn saved_networks_rejoin_and_can_be_forgotten() {
    let s = status();
    let mut n = NetScreen::new();
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    for _ in 0..3 {
        n.input(NetKey::Down, &s);
    }
    assert_eq!(
        n.input(NetKey::Y, &s),
        NetIntent::Forget("Grandma's".into())
    );
    assert_eq!(
        n.input(NetKey::A, &s),
        NetIntent::JoinSaved("Grandma's".into())
    );
}

fn type_text(n: &mut NetScreen, s: &NetStatus, text: &str) {
    for c in text.chars() {
        // Find the key on any layer by walking to it, the way a person would.
        for _ in 0..3 {
            if let Some((row, col)) = find(&n.keyboard, c) {
                while n.keyboard.row != row {
                    n.input(NetKey::Down, s);
                }
                while n.keyboard.col != col {
                    n.input(NetKey::Right, s);
                }
                n.input(NetKey::A, s);
                break;
            }
            match n.keyboard.layer {
                NetLayer::Lower => n.input(NetKey::Y, s),
                NetLayer::Upper => n.input(NetKey::Select, s),
                NetLayer::Symbols => n.input(NetKey::Select, s),
            };
        }
    }
}

fn find(kb: &NetKeyboard, c: char) -> Option<(usize, usize)> {
    (0..4).find_map(|row| {
        (0..kb.row_len(row))
            .find_map(|col| (kb.cap(row, col) == NetCap::Char(c)).then_some((row, col)))
    })
}

#[test]
fn a_password_typed_with_the_dpad_joins_the_network() {
    let s = status();
    let mut n = NetScreen::new();
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    let password = "Pa$$ w0rd!";
    type_text(&mut n, &s, password);
    assert_eq!(
        n.keyboard.text,
        password.replace(' ', ""),
        "space is not on the letter rows"
    );
    n.input(NetKey::X, &s);
    assert!(n.keyboard.text.ends_with(' '));
    n.input(NetKey::B, &s);
    n.input(NetKey::B, &s);
    assert_eq!(n.keyboard.text, "Pa$$w0rd");
    assert_eq!(
        n.input(NetKey::Start, &s),
        NetIntent::Join {
            ssid: "Neighbour 5G".into(),
            psk: "Pa$$w0rd".into(),
            hidden: false
        }
    );
}

#[test]
fn a_short_password_is_refused_with_a_reason() {
    let s = status();
    let mut n = NetScreen::new();
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    type_text(&mut n, &s, "short");
    assert_eq!(n.input(NetKey::Start, &s), NetIntent::None);
    assert!(n.keyboard.error.is_some());
    // B deletes, and on an empty field goes back to the list.
    for _ in 0..6 {
        n.input(NetKey::B, &s);
    }
    assert_eq!(n.page, NetPage::Networks);
}

#[test]
fn a_hidden_network_asks_for_its_name_then_password() {
    let s = status();
    let mut n = NetScreen::new();
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    n.input(NetKey::Up, &s);
    n.input(NetKey::A, &s);
    type_text(&mut n, &s, "attic");
    n.input(NetKey::Start, &s);
    assert!(
        matches!(&n.page, NetPage::Keyboard(slot_ui::KeyTarget::HiddenPassword { ssid }) if ssid == "attic")
    );
    assert_eq!(
        n.input(NetKey::Start, &s),
        NetIntent::Join {
            ssid: "attic".into(),
            psk: String::new(),
            hidden: true
        }
    );
}

#[test]
fn moving_down_lands_on_the_key_below() {
    let mut kb = NetKeyboard::new();
    kb.row = 1;
    kb.col = 9; // p
    kb.move_by(NetKey::Down); // to the 10-key row
    assert_eq!(kb.selected(), NetCap::Char('@'));
    kb.move_by(NetKey::Down);
    kb.move_by(NetKey::Down);
    assert!(matches!(kb.selected(), NetCap::Act(_)));
    kb.move_by(NetKey::Down);
    assert_eq!(kb.row, 0, "down from the bottom wraps to the top");
}

#[test]
fn every_face_renders_something() {
    for f in [
        NetFace::Title("Network".into()),
        NetFace::Label("Wi-Fi".into(), true),
        NetFace::Value("Café ünïcode".into(), false),
        NetFace::Small("Looking…".into()),
        NetFace::Mono("http://192.168.1.42:8384".into()),
        NetFace::Key("Q".into(), true),
        NetFace::Field("hunter2".into()),
        NetFace::Hint("A", "Join".into()),
        NetFace::Lock(true),
        NetFace::Qr("MFZWI3D-BONSGYC".into()),
    ] {
        let face = net_face(&f);
        assert!(face.w > 0 && face.h > 0, "{f:?} is empty");
        assert!(face.rgba.chunks(4).any(|p| p[3] > 0), "{f:?} has no ink");
    }
}

#[test]
fn a_long_name_is_cut_with_an_ellipsis() {
    let long = net_face(&NetFace::Value("x".repeat(200), true));
    assert!(long.w <= 342);
}

#[test]
fn the_device_qr_code_is_scannable_size() {
    let id = "MFZWI3D-BONSGYC-YLTMRWG-C43ENR5-QXGZDMM-FZWI3DP-BONSGYY-LTMRWAD";
    let q = qr_face(id);
    assert_eq!(q.w, q.h);
    assert!((150..=260).contains(&q.w), "QR is {} px", q.w);
}

// ----------------------------------------------------------------------------
// A software compositor, so the pages can be looked at without a GPU.

struct Cpu {
    faces: Vec<UndoFace>,
}

impl Cpu {
    fn fill(&mut self, net: &mut NetFaces) {
        for f in net.take_wanted() {
            let face = net_face(&f);
            let id = TexId::from_raw(self.faces.len());
            net.ready.insert(f, (id, face.w, face.h));
            self.faces.push(face);
        }
    }

    fn render(&self, w: usize, h: usize, draws: &[Draw]) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        for p in px.chunks_mut(4) {
            p.copy_from_slice(&[0x24, 0x24, 0x2a, 0xff]);
        }
        let blend = |px: &mut [u8], x: i32, y: i32, c: [f32; 4]| {
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                return;
            }
            let at = (y as usize * w + x as usize) * 4;
            for i in 0..3 {
                let d = px[at + i] as f32 / 255.0;
                px[at + i] = ((c[i] * c[3] + d * (1.0 - c[3])) * 255.0).round() as u8;
            }
        };
        for d in draws {
            match *d {
                Draw::Rect {
                    x,
                    y,
                    w: rw,
                    h: rh,
                    colour,
                } => {
                    for yy in y as i32..(y + rh) as i32 {
                        for xx in x as i32..(x + rw) as i32 {
                            blend(&mut px, xx, yy, colour);
                        }
                    }
                }
                Draw::Tex { x, y, tex, .. } => {
                    let id = self
                        .faces
                        .iter()
                        .enumerate()
                        .find(|(i, _)| TexId::from_raw(*i) == tex)
                        .map(|(i, _)| i);
                    let Some(f) = id.map(|i| &self.faces[i]) else {
                        continue;
                    };
                    for fy in 0..f.h {
                        for fx in 0..f.w {
                            let o = ((fy * f.w + fx) * 4) as usize;
                            let c = [
                                f.rgba[o] as f32 / 255.0,
                                f.rgba[o + 1] as f32 / 255.0,
                                f.rgba[o + 2] as f32 / 255.0,
                                f.rgba[o + 3] as f32 / 255.0,
                            ];
                            blend(&mut px, x as i32 + fx as i32, y as i32 + fy as i32, c);
                        }
                    }
                }
                _ => {}
            }
        }
        px
    }
}

fn ach() -> slot_ui::AchView {
    slot_ui::AchView {
        enabled: true,
        user: Some("Player1".into()),
        logged_in: true,
        state: "Logged in".into(),
        pending: 2,
        game: Some("12 of 40 unlocked".into()),
    }
}

#[test]
fn tailscale_logs_in_from_its_page_and_out_again() {
    let mut s = status();
    let mut n = NetScreen::new();
    for _ in 0..4 {
        n.input(NetKey::Down, &s);
    }
    assert_eq!(n.overview_row(), OverviewRow::Tailscale);
    n.input(NetKey::A, &s);
    assert_eq!(n.page, NetPage::Tailscale);
    assert_eq!(n.input(NetKey::Right, &s), NetIntent::SetTailscale(false));
    assert_eq!(
        slot_ui::TailRow::shown(&s),
        [
            slot_ui::TailRow::Tailscale,
            slot_ui::TailRow::Status,
            slot_ui::TailRow::Address,
            slot_ui::TailRow::Name,
            slot_ui::TailRow::LogOut
        ]
    );
    n.tail_row = 4;
    assert_eq!(n.input(NetKey::A, &s), NetIntent::TailLogout);
    s.tail = slot_net::TailState::NeedsLogin { url: None };
    n.tail_row = 2;
    assert_eq!(n.tail_row(&s), slot_ui::TailRow::LogIn);
    n.input(NetKey::A, &s);
    assert_eq!(n.page, NetPage::TailLogin);
    n.input(NetKey::B, &s);
    assert_eq!(n.page, NetPage::Tailscale);
}

#[test]
fn an_achievements_account_is_typed_in_two_steps() {
    let s = status();
    let mut n = NetScreen::new();
    n.page = NetPage::Achievements;
    n.ach = slot_ui::AchView {
        enabled: true,
        state: "Not logged in".into(),
        ..Default::default()
    };
    assert_eq!(n.input(NetKey::A, &s), NetIntent::SetAchievements(false));
    n.input(NetKey::Down, &s);
    n.input(NetKey::A, &s);
    assert_eq!(n.page, NetPage::Keyboard(slot_ui::KeyTarget::AchUser));
    assert_eq!(n.title(), "User Name");
    type_text(&mut n, &s, "Player1");
    n.input(NetKey::Start, &s);
    assert_eq!(
        n.page,
        NetPage::Keyboard(slot_ui::KeyTarget::AchPassword {
            user: "Player1".into()
        })
    );
    type_text(&mut n, &s, "hunter2");
    assert_eq!(
        n.input(NetKey::Start, &s),
        NetIntent::AchLogin {
            user: "Player1".into(),
            password: "hunter2".into()
        }
    );
    assert_eq!(n.page, NetPage::Achievements);
    n.ach = ach();
    n.ach_row = 4;
    assert_eq!(n.ach_row(), slot_ui::AchRow::Saved);
    assert_eq!(n.input(NetKey::A, &s), NetIntent::AchSend);
    n.ach_row = 5;
    assert_eq!(n.input(NetKey::A, &s), NetIntent::AchLogout);
}

#[test]
fn a_notice_slides_in_with_its_badge() {
    let dir = std::env::temp_dir().join(format!("slot-badge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let badge = dir.join("00234.png");
    {
        let f = std::fs::File::create(&badge).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), 64, 64);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut data = vec![0u8; 64 * 64 * 3];
        for (i, p) in data.chunks_mut(3).enumerate() {
            p.copy_from_slice(&[(i % 64 * 4) as u8, 0x80, 0xff - (i / 64 * 4) as u8]);
        }
        enc.write_header().unwrap().write_image_data(&data).unwrap();
    }
    let canvas = Canvas { w: 640.0, h: 480.0 };
    let mut faces = NetFaces::default();
    let mut cpu = Cpu { faces: Vec::new() };
    let mut draws = Vec::new();
    for _ in 0..2 {
        draws.clear();
        slot_ui::draw_notice(
            canvas,
            "Unlocked · 10 points",
            "Champion of Hoenn",
            Some("Defeat the Elite Four"),
            Some(&badge),
            1.0,
            &faces,
            &mut draws,
        );
        cpu.fill(&mut faces);
    }
    assert!(faces.take_wanted().is_empty(), "faces still missing");
    let px = cpu.render(640, 480, &draws);
    if let Some(dir) = std::env::var_os("SLOT_NET_PREVIEW") {
        let path = std::path::Path::new(&dir).join("notice.png");
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 640, 480);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut hidden = Vec::new();
    slot_ui::draw_notice(canvas, "a", "b", None, None, 0.0, &faces, &mut hidden);
    let Some(Draw::Rect { y, h, .. }) = hidden.get(1) else {
        panic!("no banner");
    };
    assert!(y + h <= 0.0, "a notice not yet shown sits above the screen");
}

fn shoot(name: &str, canvas: Canvas, screen: &NetScreen, s: &NetStatus) -> Vec<u8> {
    let mut faces = NetFaces::default();
    let mut cpu = Cpu { faces: Vec::new() };
    let mut draws = Vec::new();
    for _ in 0..2 {
        draws.clear();
        screen.draw(canvas, s, &faces, &mut draws);
        let title = faces
            .get(NetFace::Title(screen.title().into()))
            .map(|t| t.0);
        let (hints, hint_faces, ways_out) = screen.hint_faces(s, &faces);
        slot_ui::draw_album_chrome(
            canvas,
            slot_ui::AlbumChrome {
                title,
                hints: &hints,
                hint_faces: &hint_faces,
                ways_out,
                dots: None,
            },
            None,
            slot_ui::Printed::default(),
            None,
            slot_ui::Printed::default(),
            &mut draws,
        );
        cpu.fill(&mut faces);
    }
    let (hints, hint_faces, _) = screen.hint_faces(s, &faces);
    assert_eq!(hints.len(), hint_faces.len(), "{name}: hint faces missing");
    assert!(
        faces.take_wanted().is_empty(),
        "{name}: faces still missing on the second frame"
    );
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    let px = cpu.render(w, h, &draws);
    if let Some(dir) = std::env::var_os("SLOT_NET_PREVIEW") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
    }
    px
}

#[test]
fn every_page_draws_on_both_screen_widths() {
    let s = status();
    slot_ui::set_online(true);
    for (tag, canvas) in [
        ("720", Canvas { w: 720.0, h: 480.0 }),
        ("640", Canvas { w: 640.0, h: 480.0 }),
    ] {
        let mut n = NetScreen::new();
        n.input(NetKey::Down, &s);
        shoot(&format!("{tag}-overview"), canvas, &n, &s);
        n.input(NetKey::A, &s);
        n.input(NetKey::Down, &s);
        n.input(NetKey::Down, &s);
        shoot(&format!("{tag}-networks"), canvas, &n, &s);
        n.input(NetKey::A, &s);
        type_text(&mut n, &s, "Hunter2");
        n.input(NetKey::Down, &s);
        shoot(&format!("{tag}-keyboard"), canvas, &n, &s);
        n.input(NetKey::Select, &s);
        shoot(&format!("{tag}-symbols"), canvas, &n, &s);
        let mut n = NetScreen::new();
        n.ach = ach();
        for _ in 0..4 {
            n.input(NetKey::Down, &s);
        }
        n.input(NetKey::A, &s);
        shoot(&format!("{tag}-tailscale"), canvas, &n, &s);
        let mut login = s.clone();
        login.tail = slot_net::TailState::NeedsLogin {
            url: Some("https://login.tailscale.com/a/1a2b3c4d5e6f".into()),
        };
        n.tail_row = 2;
        n.input(NetKey::A, &login);
        assert_eq!(n.page, NetPage::TailLogin);
        let px = shoot(&format!("{tag}-tail-login"), canvas, &n, &login);
        assert!(
            px.chunks(4)
                .filter(|p| p[0] == 0xff && p[1] == 0xff)
                .count()
                > 10_000,
            "{tag}: no code"
        );
        let mut n = NetScreen::new();
        n.ach = ach();
        for _ in 0..5 {
            n.input(NetKey::Down, &s);
        }
        shoot(&format!("{tag}-overview-lower"), canvas, &n, &s);
        n.input(NetKey::A, &s);
        assert_eq!(n.page, NetPage::Achievements);
        shoot(&format!("{tag}-achievements"), canvas, &n, &s);
        let mut n = NetScreen::new();
        for _ in 0..3 {
            n.input(NetKey::Down, &s);
        }
        n.input(NetKey::A, &s);
        shoot(&format!("{tag}-syncthing"), canvas, &n, &s);
        n.input(NetKey::Down, &s);
        n.input(NetKey::Down, &s);
        n.input(NetKey::A, &s);
        let px = shoot(&format!("{tag}-sync"), canvas, &n, &s);
        let white = px
            .chunks(4)
            .filter(|p| p[0] == 0xff && p[1] == 0xff)
            .count();
        assert!(white > 10_000, "{tag}: no QR code on the Syncthing page");
    }
}

#[test]
fn keys_fit_inside_the_screen() {
    for w in [640.0, 720.0] {
        let canvas = Canvas { w, h: 480.0 };
        let kb = NetKeyboard::new();
        for row in 0..5 {
            for col in 0..kb.row_len(row) {
                let b = slot_ui::net_key_box(&kb, canvas, 200.0, row, col);
                assert!(
                    b.x >= 0.0 && b.x + b.w <= w,
                    "key {row},{col} at {b:?} on {w}"
                );
            }
        }
    }
}
