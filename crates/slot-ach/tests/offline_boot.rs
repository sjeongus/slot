//! Starting with no network at all: achievements stay on, the account logs in from what the
//! card saved while online, and a saved game loads. A file of its own, because rcheevos'
//! client is one per process.

use std::path::Path;
use std::time::{Duration, Instant};

use slot_ach::transport::{Reply, Transport};
use slot_ach::{Account, Event, Session, Settings};
use slot_retro::{AvInfo, ButtonMask, CoreError, MemoryDescriptor, RetroCore};

fn offline() -> bool {
    false
}

struct NoNetwork;

impl Transport for NoNetwork {
    fn post(&self, _url: &str, _body: &str, _ct: &str) -> Reply {
        Reply::Unreachable
    }
    fn fetch(&self, _url: &str, _dest: &Path) -> bool {
        false
    }
}

struct Ram(Box<[u8; 0x8000]>);

impl RetroCore for Ram {
    fn load(&mut self, _rom: &Path) -> Result<(), CoreError> {
        Ok(())
    }
    fn run_frame(&mut self, _input: ButtonMask) {}
    fn video_xrgb8888(&self) -> &[u8] {
        &[]
    }
    fn take_audio(&mut self) -> Vec<i16> {
        Vec::new()
    }
    fn serialize(&mut self) -> Result<Vec<u8>, CoreError> {
        Ok(Vec::new())
    }
    fn unserialize(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn save_ram(&self) -> Option<Vec<u8>> {
        None
    }
    fn load_save_ram(&mut self, _data: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn av_info(&self) -> AvInfo {
        AvInfo {
            fps: 60.0,
            sample_rate: 48_000.0,
        }
    }
    fn memory_map(&self) -> Vec<MemoryDescriptor> {
        vec![MemoryDescriptor {
            flags: 1 << 2,
            ptr: self.0.as_ptr() as *mut u8,
            offset: 0,
            start: 0x0300_0000,
            select: 0xFF00_0000,
            disconnect: 0,
            len: 0x8000,
        }]
    }
}

const LOGIN: &str = r#"{"Success":true,"User":"User","Token":"ApiToken","Score":1,"SoftcoreScore":1,"Messages":0,"Permissions":1,"AccountType":"Registered"}"#;

const SETS: &str = concat!(
    r#"{"Success":true,"GameId":1234,"Title":"Sample Game","ConsoleId":5,"#,
    r#""ImageIconUrl":"http://server/Images/112233.png","RichPresenceGameId":1234,"RichPresencePatch":"","#,
    r#""Sets":[{"AchievementSetId":1111,"GameId":1234,"Title":null,"Type":"core","#,
    r#""ImageIconUrl":"http://server/Images/112233.png","Achievements":["#,
    r#"{"ID":5501,"Title":"Ach1","Description":"Desc1","Flags":3,"Points":5,"MemAddr":"0xH0001=3","Author":"User1","BadgeName":"00234","Created":1367266583,"Modified":1376929305}"#,
    r#"],"Leaderboards":[]}]}"#
);

#[test]
fn with_no_network_achievements_stay_on_and_a_saved_game_loads() {
    let dir = std::env::temp_dir().join(format!("slot-ach-offline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Roms")).unwrap();
    let rom = dir.join("Roms").join("Sample.gba");
    std::fs::write(&rom, vec![7u8; 4096]).unwrap();
    // What an earlier online session left on the card.
    Settings {
        enabled: true,
        user: "User".into(),
        token: "ApiToken".into(),
    }
    .save(&dir)
    .unwrap();
    let store = slot_ach::store::Store::new(&dir);
    store.save_login("User", LOGIN);
    let hash = format!("{:x}", md5_of(&std::fs::read(&rom).unwrap()));
    store.save_sets(&hash, SETS);

    slot_ach::start_with(&dir, Box::new(NoNetwork), offline);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = slot_ach::status();
        assert!(s.enabled, "achievements turned themselves off: {s:?}");
        if matches!(s.account, Account::LoggedIn { offline: true }) {
            break;
        }
        assert!(Instant::now() < deadline, "no offline login: {s:?}");
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut core = Ram(Box::new([0u8; 0x8000]));
    let mut session = Session::begin(&rom, &mut core).expect("a session with no network");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while !seen.iter().any(|e| matches!(e, Event::GameLoaded { .. })) {
        session.frame();
        seen.extend(slot_ach::take_events());
        assert!(Instant::now() < deadline, "no game: {seen:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(seen.iter().any(|e| matches!(
        e,
        Event::GameLoaded {
            offline: true,
            total: 1,
            ..
        }
    )));
    assert!(slot_ach::status().enabled);
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
}

/// MD5, enough of it to name the ROM the way RetroAchievements hashes a GBA game.
fn md5_of(data: &[u8]) -> u128 {
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(s[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    u128::from_be_bytes(out)
}
