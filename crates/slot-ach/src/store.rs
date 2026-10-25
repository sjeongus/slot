//! What slot keeps on the card so achievements work offline, in `Config/achievements/`:
//!
//! - `cache/login-<user>.json`: the last login answer, so the account works offline.
//! - `cache/sets-<hash>.json`: each game's achievement set, saved the first time it is
//!   played online.
//! - `unlocks/<hash>.txt`: what this account has unlocked in each game, so an offline session
//!   starts with them unlocked.
//! - `pending.txt`: unlocks earned offline, waiting to be sent, oldest first.
//! - `badges/<name>.png`: achievement and game images.
//! - `hashes.txt`: each ROM's RetroAchievements hash, by path, size and time, so a library is
//!   hashed once.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub id: u32,
    /// Unix seconds when it was earned.
    pub when: u64,
    pub hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unlocks {
    /// (achievement id, unix seconds)
    pub softcore: Vec<(u32, u64)>,
    pub hardcore: Vec<(u32, u64)>,
}

pub struct Store {
    dir: PathBuf,
    /// `hashes.txt`, read once: file key to hash.
    index: std::sync::Mutex<Option<std::collections::HashMap<String, String>>>,
}

/// Hashes and user names become file names; keep only what is safe in one.
fn safe(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect::<String>()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

/// A ROM's path, size and modification time: unchanged means its hash is too.
fn file_key(rom: &Path) -> Option<String> {
    let meta = std::fs::metadata(rom).ok()?;
    let secs = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}\t{}\t{secs}", rom.to_string_lossy(), meta.len()))
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl Store {
    pub fn new(root: &Path) -> Store {
        Store {
            dir: root.join("Config").join("achievements"),
            index: std::sync::Mutex::new(None),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn login_path(&self, user: &str) -> PathBuf {
        self.dir
            .join("cache")
            .join(format!("login-{}.json", safe(user)))
    }

    pub fn save_login(&self, user: &str, body: &str) {
        let _ = write_atomic(&self.login_path(user), body);
    }

    pub fn login(&self, user: &str) -> Option<String> {
        std::fs::read_to_string(self.login_path(user)).ok()
    }

    pub fn forget_login(&self, user: &str) {
        let _ = std::fs::remove_file(self.login_path(user));
    }

    fn sets_path(&self, hash: &str) -> PathBuf {
        self.dir
            .join("cache")
            .join(format!("sets-{}.json", safe(hash)))
    }

    pub fn save_sets(&self, hash: &str, body: &str) {
        let _ = write_atomic(&self.sets_path(hash), body);
    }

    pub fn sets(&self, hash: &str) -> Option<String> {
        std::fs::read_to_string(self.sets_path(hash)).ok()
    }

    pub fn has_sets(&self, hash: &str) -> bool {
        self.sets_path(hash).is_file()
    }

    /// The game a saved set belongs to; 0 if RetroAchievements did not know the ROM.
    pub fn game_id(&self, hash: &str) -> Option<u32> {
        let j = slot_net::json::Json::parse(&self.sets(hash)?)?;
        match j.get("GameId") {
            slot_net::json::Json::Num(n) => Some(*n as u32),
            _ => Some(0),
        }
    }

    pub fn has_unlocks(&self, hash: &str) -> bool {
        self.unlocks_path(hash).is_file()
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("hashes.txt")
    }

    fn with_index<R>(
        &self,
        f: impl FnOnce(&mut std::collections::HashMap<String, String>) -> R,
    ) -> R {
        let mut index = self.index.lock().unwrap_or_else(|p| p.into_inner());
        let map = index.get_or_insert_with(|| {
            std::fs::read_to_string(self.index_path())
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    let (k, hash) = l.rsplit_once('\t')?;
                    Some((k.to_string(), hash.to_string()))
                })
                .collect()
        });
        f(map)
    }

    /// The saved hash of `rom`, if the file has not changed since it was hashed.
    pub fn known_hash(&self, rom: &Path) -> Option<String> {
        let key = file_key(rom)?;
        self.with_index(|m| m.get(&key).cloned())
    }

    pub fn remember_hash(&self, rom: &Path, hash: &str) {
        let Some(key) = file_key(rom) else {
            return;
        };
        let path = format!("{}\t", rom.to_string_lossy());
        let text = self.with_index(|m| {
            // One entry per ROM: drop what an older copy of this file left.
            m.retain(|k, _| !k.starts_with(&path));
            m.insert(key, hash.to_string());
            let mut lines: Vec<String> = m.iter().map(|(k, h)| format!("{k}\t{h}\n")).collect();
            lines.sort();
            lines.concat()
        });
        let _ = write_atomic(&self.index_path(), &text);
    }

    fn unlocks_path(&self, hash: &str) -> PathBuf {
        self.dir.join("unlocks").join(format!("{}.txt", safe(hash)))
    }

    pub fn unlocks(&self, hash: &str) -> Unlocks {
        let mut u = Unlocks::default();
        let text = std::fs::read_to_string(self.unlocks_path(hash)).unwrap_or_default();
        for line in text.lines() {
            let mut f = line.split_whitespace();
            let (Some(kind), Some(id), Some(when)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(id), Ok(when)) = (id.parse(), when.parse()) else {
                continue;
            };
            match kind {
                "h" => u.hardcore.push((id, when)),
                _ => u.softcore.push((id, when)),
            }
        }
        u
    }

    fn save_unlocks(&self, hash: &str, u: &Unlocks) {
        let mut text = String::new();
        for (id, when) in &u.softcore {
            text += &format!("s {id} {when}\n");
        }
        for (id, when) in &u.hardcore {
            text += &format!("h {id} {when}\n");
        }
        let _ = write_atomic(&self.unlocks_path(hash), &text);
    }

    /// Takes in what the server says is unlocked, keeping anything it does not know of yet.
    pub fn merge_unlocks(&self, hash: &str, server: &Unlocks) {
        let mut u = self.unlocks(hash);
        for (list, from) in [
            (&mut u.softcore, &server.softcore),
            (&mut u.hardcore, &server.hardcore),
        ] {
            for (id, when) in from {
                if !list.iter().any(|(i, _)| i == id) {
                    list.push((*id, *when));
                }
            }
        }
        self.save_unlocks(hash, &u);
    }

    pub fn add_unlock(&self, hash: &str, id: u32, when: u64) {
        let mut u = self.unlocks(hash);
        if !u.softcore.iter().any(|(i, _)| *i == id) {
            u.softcore.push((id, when));
            self.save_unlocks(hash, &u);
        }
    }

    fn pending_path(&self) -> PathBuf {
        self.dir.join("pending.txt")
    }

    pub fn pending(&self) -> Vec<Pending> {
        std::fs::read_to_string(self.pending_path())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                Some(Pending {
                    id: f.next()?.parse().ok()?,
                    when: f.next()?.parse().ok()?,
                    hash: f.next()?.to_string(),
                })
            })
            .collect()
    }

    fn save_pending(&self, all: &[Pending]) {
        let text: String = all
            .iter()
            .map(|p| format!("{} {} {}\n", p.id, p.when, p.hash))
            .collect();
        let _ = write_atomic(&self.pending_path(), &text);
    }

    /// Queues an unlock earned offline. The first one is kept if it is earned again, as after
    /// loading an older state.
    pub fn push_pending(&self, p: Pending) {
        let mut all = self.pending();
        if all.iter().any(|q| q.id == p.id) {
            return;
        }
        self.add_unlock(&p.hash, p.id, p.when);
        all.push(p);
        self.save_pending(&all);
    }

    pub fn drop_pending(&self, id: u32) {
        let mut all = self.pending();
        all.retain(|p| p.id != id);
        self.save_pending(&all);
    }

    pub fn badge_path(&self, name: &str) -> PathBuf {
        self.dir.join("badges").join(format!("{}.png", safe(name)))
    }

    pub fn badge(&self, name: &str) -> Option<PathBuf> {
        Some(self.badge_path(name)).filter(|p| !name.is_empty() && p.is_file())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn store() -> (tempdir::Dir, Store) {
        let d = tempdir::Dir::new();
        let s = Store::new(&d.0);
        (d, s)
    }

    pub mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let p = std::env::temp_dir().join(format!(
                    "slot-ach-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&p);
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn an_unlock_earned_twice_offline_is_queued_once() {
        let (_d, s) = store();
        let p = |when| Pending {
            id: 7,
            when,
            hash: "abc".into(),
        };
        s.push_pending(p(100));
        s.push_pending(p(200));
        assert_eq!(s.pending(), [p(100)]);
        assert_eq!(s.unlocks("abc").softcore, [(7, 100)]);
        s.drop_pending(7);
        assert!(s.pending().is_empty());
        assert_eq!(s.unlocks("abc").softcore, [(7, 100)], "still unlocked");
    }

    #[test]
    fn the_servers_unlocks_merge_with_ours() {
        let (_d, s) = store();
        s.add_unlock("abc", 1, 10);
        s.merge_unlocks(
            "abc",
            &Unlocks {
                softcore: vec![(1, 5), (2, 20)],
                hardcore: vec![(3, 30)],
            },
        );
        let u = s.unlocks("abc");
        assert_eq!(u.softcore, [(1, 10), (2, 20)]);
        assert_eq!(u.hardcore, [(3, 30)]);
    }

    #[test]
    fn a_rom_is_hashed_once_until_it_changes() {
        let (d, s) = store();
        let rom = d.0.join("Game.gba");
        std::fs::write(&rom, b"one").unwrap();
        assert_eq!(s.known_hash(&rom), None);
        s.remember_hash(&rom, "aaa");
        assert_eq!(s.known_hash(&rom).as_deref(), Some("aaa"));
        assert_eq!(
            Store::new(&d.0).known_hash(&rom).as_deref(),
            Some("aaa"),
            "kept on the card"
        );
        std::fs::write(&rom, b"longer now").unwrap();
        assert_eq!(s.known_hash(&rom), None, "a changed file is hashed again");
        s.remember_hash(&rom, "bbb");
        let index = std::fs::read_to_string(s.dir().join("hashes.txt")).unwrap();
        assert_eq!(index.lines().count(), 1, "one line per ROM: {index}");
    }

    #[test]
    fn names_from_the_server_cannot_leave_the_folder() {
        let (_d, s) = store();
        assert_eq!(
            s.badge_path("../../etc/passwd"),
            s.dir().join("badges").join("etcpasswd.png")
        );
        s.save_login("Some/User", "{}");
        assert_eq!(s.login("Some/User").as_deref(), Some("{}"));
    }
}
