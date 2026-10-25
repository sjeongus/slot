//! Every request rcheevos makes comes through here. Online, it goes to RetroAchievements and
//! slot keeps what it needs to play offline later: the login, each game's achievement set and
//! the account's unlocks. Offline, slot answers from those, and an unlock is queued on the
//! card and answered as a success, so the game carries on as if it were online. The queue is
//! sent when slot is back online, each unlock with how long ago it was earned, as
//! RAOfflineProxy does. Only softcore unlocks are queued; slot plays softcore only.

use std::time::{SystemTime, UNIX_EPOCH};

use slot_net::json::Json;

use crate::store::{Pending, Store, Unlocks};
use crate::transport::{Reply, Transport};

/// RetroAchievements accepts an unlock at most 14 days after it was earned.
pub const BACKDATE_LIMIT: u64 = 14 * 24 * 3600;

/// rcheevos' "no answer, try again" status (RC_API_SERVER_RESPONSE_RETRYABLE_CLIENT_ERROR).
pub const RETRY: i32 = -2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: i32,
    pub body: String,
    /// The answer came from the card, not the server.
    pub offline: bool,
}

impl Answer {
    fn local(body: impl Into<String>) -> Answer {
        Answer {
            status: 200,
            body: body.into(),
            offline: true,
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The `key=value&...` pairs of a request body, decoded.
pub fn params(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((decode(k), decode(v)))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param<'a>(p: &'a [(String, String)], key: &str) -> &'a str {
    p.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn succeeded(body: &str) -> bool {
    Json::parse(body).is_some_and(|j| j.get("Success").as_bool() == Some(true))
}

fn error_of(body: &str) -> String {
    Json::parse(body)
        .and_then(|j| j.get("Error").as_str().map(str::to_string))
        .unwrap_or_default()
}

fn unlock_list(j: &Json, key: &str) -> Vec<(u32, u64)> {
    j.get(key)
        .as_array()
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let num = |k: &str| match e.get(k) {
                Json::Num(n) => Some(*n as u64),
                Json::Str(s) => s.parse().ok(),
                _ => None,
            };
            Some((num("ID")? as u32, num("When").unwrap_or(0)))
        })
        .collect()
}

fn list_json(list: &[(u32, u64)]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|(id, when)| format!("{{\"ID\":{id},\"When\":{when}}}"))
        .collect();
    format!("[{}]", items.join(","))
}

fn offline_error(message: &str) -> Answer {
    Answer::local(format!(
        "{{\"Success\":false,\"Error\":{}}}",
        slot_net::json::quote(message)
    ))
}

pub struct Proxy {
    pub store: Store,
    transport: Box<dyn Transport>,
}

/// What became of one unlock sent from the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    Accepted,
    /// RetroAchievements refused the account's token: log in again.
    Refused(String),
    /// Refused for this unlock alone (retired, say); it leaves the queue.
    Dropped(String),
    Unreachable,
}

impl Proxy {
    pub fn new(store: Store, transport: Box<dyn Transport>) -> Proxy {
        Proxy { store, transport }
    }

    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    /// Answers one rcheevos request. `online` says whether to try the server at all.
    pub fn serve(&self, url: &str, body: &str, content_type: &str, online: bool) -> Answer {
        let p = params(body);
        let r = param(&p, "r");
        if online {
            if let Reply::Answered {
                status,
                body: answer,
            } = self.transport.post(url, body, content_type)
            {
                return self.learn(r, &p, i32::from(status), answer);
            }
        }
        self.answer_offline(r, &p)
    }

    /// Keeps what an online answer tells us, and passes it on.
    fn learn(&self, r: &str, p: &[(String, String)], status: i32, body: String) -> Answer {
        let ok = succeeded(&body);
        match r {
            "login2" if ok => self.store.save_login(param(p, "u"), &body),
            "achievementsets" | "patch" if ok => {
                let hash = param(p, "m");
                if !hash.is_empty() {
                    self.store.save_sets(hash, &body);
                }
            }
            "startsession" if ok => {
                let hash = param(p, "m");
                if let (false, Some(j)) = (hash.is_empty(), Json::parse(&body)) {
                    self.store.merge_unlocks(
                        hash,
                        &Unlocks {
                            softcore: unlock_list(&j, "Unlocks"),
                            hardcore: unlock_list(&j, "HardcoreUnlocks"),
                        },
                    );
                    // Unlocks still waiting to be sent count as unlocked already.
                    if self.store.pending().iter().any(|q| q.hash == hash) {
                        return Answer {
                            status,
                            body: self.session(hash, &j),
                            offline: false,
                        };
                    }
                }
            }
            "awardachievement" => {
                let id: u32 = param(p, "a").parse().unwrap_or(0);
                if ok || error_of(&body).starts_with("User already has") {
                    self.store.add_unlock(param(p, "m"), id, now());
                }
            }
            _ => {}
        }
        Answer {
            status,
            body,
            offline: false,
        }
    }

    /// A start-session answer from what the card knows, with queued unlocks included.
    fn session(&self, hash: &str, server: &Json) -> String {
        let mut u = self.store.unlocks(hash);
        for q in self.store.pending().iter().filter(|q| q.hash == hash) {
            if !u.softcore.iter().any(|(i, _)| *i == q.id) {
                u.softcore.push((q.id, q.when));
            }
        }
        let server_now = match server.get("ServerNow") {
            Json::Num(n) => *n as u64,
            _ => now(),
        };
        format!(
            "{{\"Success\":true,\"Unlocks\":{},\"HardcoreUnlocks\":{},\"ServerNow\":{server_now}}}",
            list_json(&u.softcore),
            list_json(&u.hardcore)
        )
    }

    fn answer_offline(&self, r: &str, p: &[(String, String)]) -> Answer {
        match r {
            "login2" => match self.store.login(param(p, "u")) {
                Some(body) => Answer::local(body),
                None => offline_error(
                    "Offline, and this account has not logged in on this device online yet",
                ),
            },
            "achievementsets" | "patch" => match self.store.sets(param(p, "m")) {
                Some(body) => Answer::local(body),
                None => offline_error("Offline, and this game has not been played online yet"),
            },
            "startsession" => Answer::local(self.session(param(p, "m"), &Json::Null)),
            "awardachievement" if param(p, "h") != "1" => {
                let id: u32 = param(p, "a").parse().unwrap_or(0);
                self.store.push_pending(Pending {
                    id,
                    when: now(),
                    hash: param(p, "m").to_string(),
                });
                Answer::local(format!(
                    "{{\"Success\":true,\"AchievementID\":{id},\"Score\":0,\"SoftcoreScore\":0}}"
                ))
            }
            "ping" => Answer::local("{\"Success\":true}"),
            _ => Answer {
                status: RETRY,
                body: String::new(),
                offline: true,
            },
        }
    }

    /// Sends one queued unlock, built by `request` (url and body) for how long ago it was
    /// earned.
    pub fn send(&self, q: &Pending, request: Option<(String, String)>) -> Sent {
        let Some((url, body)) = request else {
            return Sent::Dropped("could not build the request".into());
        };
        match self
            .transport
            .post(&url, &body, "application/x-www-form-urlencoded")
        {
            Reply::Unreachable => Sent::Unreachable,
            Reply::Answered { status, body } => {
                let error = error_of(&body);
                if succeeded(&body) || error.starts_with("User already has") {
                    self.store.drop_pending(q.id);
                    self.store.add_unlock(&q.hash, q.id, q.when);
                    Sent::Accepted
                } else if status == 401 || status == 403 || error.to_lowercase().contains("token") {
                    Sent::Refused(error)
                } else if status >= 500 || status == 429 {
                    Sent::Unreachable
                } else {
                    self.store.drop_pending(q.id);
                    Sent::Dropped(error)
                }
            }
        }
    }
}

/// How many seconds ago `when` was, within what RetroAchievements accepts.
pub fn seconds_since(when: u64) -> u32 {
    now().saturating_sub(when).clamp(1, BACKDATE_LIMIT) as u32
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// Answers like RetroAchievements while `up`, and records what it was asked.
    #[derive(Default)]
    pub struct Fake {
        pub up: Mutex<bool>,
        pub asked: Mutex<Vec<String>>,
        pub answers: Mutex<Vec<(String, u16, String)>>,
    }

    impl Fake {
        pub fn answer(&self, r: &str, status: u16, body: &str) {
            self.answers
                .lock()
                .unwrap()
                .push((r.to_string(), status, body.to_string()));
        }
    }

    impl Transport for std::sync::Arc<Fake> {
        fn post(&self, _url: &str, body: &str, _ct: &str) -> Reply {
            self.asked.lock().unwrap().push(body.to_string());
            if !*self.up.lock().unwrap() {
                return Reply::Unreachable;
            }
            let r = param(&params(body), "r").to_string();
            let answers = self.answers.lock().unwrap();
            match answers.iter().rev().find(|(k, _, _)| *k == r) {
                Some((_, status, body)) => Reply::Answered {
                    status: *status,
                    body: body.clone(),
                },
                None => Reply::Answered {
                    status: 200,
                    body: "{\"Success\":true}".into(),
                },
            }
        }
        fn fetch(&self, _url: &str, _dest: &Path) -> bool {
            false
        }
    }

    fn proxy() -> (
        crate::store::tests::tempdir::Dir,
        std::sync::Arc<Fake>,
        Proxy,
    ) {
        let d = crate::store::tests::tempdir::Dir::new();
        let fake = std::sync::Arc::new(Fake::default());
        let p = Proxy::new(Store::new(&d.0), Box::new(fake.clone()));
        (d, fake, p)
    }

    const URL: &str = "https://retroachievements.org/dorequest.php";

    #[test]
    fn requests_are_decoded() {
        let p = params("r=login2&u=Some%20One&p=a%2Bb+c");
        assert_eq!(param(&p, "u"), "Some One");
        assert_eq!(param(&p, "p"), "a+b c");
    }

    #[test]
    fn what_was_learned_online_carries_a_game_offline() {
        let (_d, fake, p) = proxy();
        *fake.up.lock().unwrap() = true;
        let login = r#"{"Success":true,"User":"me","Token":"tok","Score":5}"#;
        fake.answer("login2", 200, login);
        fake.answer(
            "achievementsets",
            200,
            r#"{"Success":true,"GameId":42,"Sets":[]}"#,
        );
        fake.answer(
            "startsession",
            200,
            r#"{"Success":true,"Unlocks":[{"ID":1,"When":100}],"HardcoreUnlocks":[],"ServerNow":200}"#,
        );
        assert!(!p.serve(URL, "r=login2&u=me&p=secret", "", true).offline);
        p.serve(URL, "r=achievementsets&u=me&t=tok&m=abc", "", true);
        p.serve(URL, "r=startsession&u=me&t=tok&g=42&h=0&m=abc", "", true);

        *fake.up.lock().unwrap() = false;
        let a = p.serve(URL, "r=login2&u=me&t=tok", "", true);
        assert!(a.offline);
        assert_eq!(a.body, login);
        assert!(p
            .serve(URL, "r=achievementsets&u=me&t=tok&m=abc", "", true)
            .body
            .contains("GameId"));
        let award = p.serve(
            URL,
            "r=awardachievement&u=me&t=tok&a=2&h=0&m=abc&v=x",
            "",
            true,
        );
        assert!(award.offline && succeeded(&award.body));
        let session = p.serve(URL, "r=startsession&u=me&t=tok&g=42&h=0&m=abc", "", false);
        let j = Json::parse(&session.body).unwrap();
        let ids: Vec<u32> = unlock_list(&j, "Unlocks").iter().map(|u| u.0).collect();
        assert_eq!(ids, [1, 2], "the queued unlock starts unlocked");
        assert_eq!(p.store.pending().len(), 1);
    }

    #[test]
    fn offline_without_a_cache_says_why() {
        let (_d, _fake, p) = proxy();
        let a = p.serve(URL, "r=achievementsets&u=me&t=tok&m=new", "", false);
        assert!(!succeeded(&a.body));
        assert!(error_of(&a.body).contains("played online"));
        assert_eq!(
            p.serve(URL, "r=submitlbentry&u=me", "", false).status,
            RETRY
        );
    }

    #[test]
    fn queued_unlocks_are_sent_once_back_online() {
        let (_d, fake, p) = proxy();
        p.serve(
            URL,
            "r=awardachievement&u=me&t=tok&a=9&h=0&m=abc",
            "",
            false,
        );
        let q = p.store.pending()[0].clone();
        let req = || Some((URL.to_string(), "r=awardachievement&a=9&o=60".to_string()));
        assert_eq!(p.send(&q, req()), Sent::Unreachable);
        *fake.up.lock().unwrap() = true;
        fake.answer(
            "awardachievement",
            200,
            r#"{"Success":false,"Error":"User already has this achievement awarded."}"#,
        );
        assert_eq!(p.send(&q, req()), Sent::Accepted);
        assert!(p.store.pending().is_empty());
    }

    #[test]
    fn a_refused_token_keeps_the_queue() {
        let (_d, fake, p) = proxy();
        p.serve(
            URL,
            "r=awardachievement&u=me&t=tok&a=9&h=0&m=abc",
            "",
            false,
        );
        let q = p.store.pending()[0].clone();
        *fake.up.lock().unwrap() = true;
        fake.answer(
            "awardachievement",
            401,
            r#"{"Success":false,"Error":"Invalid token"}"#,
        );
        let req = Some((URL.to_string(), "r=awardachievement&a=9".to_string()));
        assert!(matches!(p.send(&q, req), Sent::Refused(_)));
        assert_eq!(p.store.pending().len(), 1);
    }

    #[test]
    fn the_backdate_is_capped_at_fourteen_days() {
        assert_eq!(seconds_since(0), BACKDATE_LIMIT as u32);
        assert!(seconds_since(now()) >= 1);
    }
}
