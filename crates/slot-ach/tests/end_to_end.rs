//! rcheevos for real, against a stand-in server and a stand-in core: log in, load a game,
//! earn an achievement online, earn another offline, and see it sent once back online.
//! One test, because rcheevos' client is one per process.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slot_ach::transport::{Reply, Transport};
use slot_ach::{Account, Event, Session};
use slot_retro::{AvInfo, ButtonMask, CoreError, MemoryDescriptor, RetroCore};

static ONLINE: AtomicBool = AtomicBool::new(true);

fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

#[derive(Default)]
struct Server {
    asked: Mutex<Vec<String>>,
}

const LOGIN: &str = r#"{"Success":true,"User":"User","Token":"ApiToken","Score":12345,"SoftcoreScore":123,"Messages":0,"Permissions":1,"AccountType":"Registered"}"#;

const SETS: &str = concat!(
    r#"{"Success":true,"GameId":1234,"Title":"Sample Game","ConsoleId":5,"#,
    r#""ImageIconUrl":"http://server/Images/112233.png","RichPresenceGameId":1234,"RichPresencePatch":"","#,
    r#""Sets":[{"AchievementSetId":1111,"GameId":1234,"Title":null,"Type":"core","#,
    r#""ImageIconUrl":"http://server/Images/112233.png","Achievements":["#,
    r#"{"ID":5501,"Title":"Ach1","Description":"Desc1","Flags":3,"Points":5,"MemAddr":"0xH0001=3","Author":"User1","BadgeName":"00234","Created":1367266583,"Modified":1376929305},"#,
    r#"{"ID":5502,"Title":"Ach2","Description":"Desc2","Flags":3,"Points":2,"MemAddr":"0xH0001=2_0xH0002=9","Author":"User1","BadgeName":"00235","Created":1376970283,"Modified":1376970283}"#,
    r#"],"Leaderboards":[]}]}"#
);

struct Shared(Arc<Server>);

impl Transport for Shared {
    fn post(&self, _url: &str, body: &str, _ct: &str) -> Reply {
        self.0.asked.lock().unwrap().push(body.to_string());
        if !online() {
            return Reply::Unreachable;
        }
        let r = body
            .split('&')
            .find_map(|kv| kv.strip_prefix("r="))
            .unwrap_or("");
        let answer = match r {
            "login2" => LOGIN.to_string(),
            "achievementsets" => SETS.to_string(),
            "unlocks" if body.contains("h=1") => r#"{"Success":true,"UserUnlocks":[]}"#.into(),
            "unlocks" => r#"{"Success":true,"UserUnlocks":[5501]}"#.into(),
            "startsession" => {
                r#"{"Success":true,"Unlocks":[],"HardcoreUnlocks":[],"ServerNow":1700000000}"#
                    .into()
            }
            "awardachievement" => {
                let a = body
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("a="))
                    .unwrap_or("0");
                format!(
                    r#"{{"Success":true,"Score":1,"SoftcoreScore":1,"AchievementID":{a},"AchievementsRemaining":1}}"#
                )
            }
            _ => r#"{"Success":true}"#.into(),
        };
        Reply::Answered {
            status: 200,
            body: answer,
        }
    }
    fn fetch(&self, _url: &str, _dest: &Path) -> bool {
        false
    }
}

/// A core with 32 KB of GBA work RAM at 0x03000000, the area RetroAchievements calls 0x0000.
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

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(
            Instant::now() < deadline,
            "waited for {what}: {:?}",
            slot_ach::status()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn frames_until(s: &mut Session, what: &str, mut ok: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    loop {
        s.frame();
        seen.extend(slot_ach::take_events());
        if ok(&seen) {
            return seen;
        }
        assert!(Instant::now() < deadline, "waited for {what}: {seen:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn earns_online_queues_offline_and_sends_later() {
    let dir = std::env::temp_dir().join(format!("slot-ach-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Roms")).unwrap();
    let rom = dir.join("Roms").join("Sample.gba");
    std::fs::write(&rom, vec![7u8; 4096]).unwrap();
    let server = Arc::new(Server::default());
    slot_ach::start_with(&dir, Box::new(Shared(server.clone())), online);

    slot_ach::login("User", "Pa$$word");
    until("login", || {
        matches!(slot_ach::status().account, Account::LoggedIn { .. })
    });
    let settings = slot_ach::Settings::load(&dir);
    assert!(settings.enabled);
    assert_eq!(settings.token, "ApiToken", "the token is kept");
    assert!(
        !std::fs::read_to_string(slot_ach::Settings::path(&dir))
            .unwrap()
            .contains("Pa$$word"),
        "the password is not"
    );

    let mut core = Ram(Box::new([0u8; 0x8000]));
    let mut session = Session::begin(&rom, &mut core).expect("a session");
    let events = frames_until(&mut session, "the game", |e| {
        e.iter().any(|e| matches!(e, Event::GameLoaded { .. }))
    });
    assert!(events.contains(&Event::GameLoaded {
        title: "Sample Game".into(),
        unlocked: 0,
        total: 2,
        badge: None,
        offline: false,
    }));

    for _ in 0..5 {
        session.frame();
    }
    core.0[1] = 3;
    let events = frames_until(&mut session, "the first unlock", |e| {
        e.iter().any(|e| matches!(e, Event::Unlocked { .. }))
    });
    assert!(events.iter().any(|e| matches!(e,
        Event::Unlocked { title, points: 5, later: false, .. } if title == "Ach1")));
    until("the award", || {
        server
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|b| b.contains("r=awardachievement") && b.contains("a=5501"))
    });

    ONLINE.store(false, Ordering::Relaxed);
    core.0[1] = 0;
    for _ in 0..5 {
        session.frame();
    }
    core.0[1] = 2;
    core.0[2] = 9;
    let events = frames_until(&mut session, "the offline unlock", |e| {
        e.iter().any(|e| matches!(e, Event::Unlocked { .. }))
    });
    assert!(events.iter().any(|e| matches!(e,
        Event::Unlocked { title, later: true, .. } if title == "Ach2")));
    // Keep the game running until the award has been answered from the card.
    let deadline = Instant::now() + Duration::from_secs(30);
    while slot_ach::status().pending == 0 {
        session.frame();
        assert!(Instant::now() < deadline, "the unlock was not queued");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(session);
    assert_eq!(slot_ach::status().game, None);

    ONLINE.store(true, Ordering::Relaxed);
    slot_ach::send_now();
    until("the queue to empty", || slot_ach::status().pending == 0);
    let asked = server.asked.lock().unwrap();
    let sent = asked
        .iter()
        .rev()
        .find(|b| b.contains("a=5502"))
        .expect("sent again");
    assert!(
        sent.contains("&o="),
        "it says how long ago it was earned: {sent}"
    );
    drop(asked);

    // A game never played here, saved ahead of time from the library while online…
    let other = dir.join("Roms").join("Other.gba");
    std::fs::write(&other, vec![9u8; 4096]).unwrap();
    slot_ach::set_library(vec![rom.clone(), other.clone()]);
    until("the library to be saved", || {
        let s = slot_ach::status();
        s.library_total == 2 && s.library_ready == 2 && !s.library_saving
    });
    // …loads offline with the unlocks this account already has.
    ONLINE.store(false, Ordering::Relaxed);
    let mut core = Ram(Box::new([0u8; 0x8000]));
    let mut session = Session::begin(&other, &mut core).expect("a session");
    let events = frames_until(&mut session, "the saved game", |e| {
        e.iter().any(|e| matches!(e, Event::GameLoaded { .. }))
    });
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::GameLoaded {
                unlocked: 1,
                total: 2,
                offline: true,
                ..
            }
        )),
        "{events:?}"
    );
    drop(session);

    // A game that was never saved says so, rather than that it has no achievements.
    let unsaved = dir.join("Roms").join("New.gba");
    std::fs::write(&unsaved, vec![3u8; 4096]).unwrap();
    let mut session = Session::begin(&unsaved, &mut core).expect("a session");
    frames_until(&mut session, "the answer", |e| e.contains(&Event::NotSaved));
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
}
