//! The one rc_client of the process, driven through `glue/glue.c`.
//!
//! Threads: rcheevos is only ever called with `Core::c` locked. Its requests go to the
//! `slot-ach` thread, which answers them through [`Proxy`] and queues the answers; whoever
//! next takes the lock hands them back to rcheevos. While a game runs that is the emulator
//! thread, once a frame, so rcheevos only ever reads game memory between frames. Otherwise
//! the `slot-ach` thread does it.
//!
//! Callbacks from C only record what happened. Anything that has to ask rcheevos something in
//! return waits until the call that raised them has returned (see [`Core::settle`]), since
//! rcheevos may still hold its own lock while it calls back.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::proxy::{seconds_since, Proxy, Sent};
use crate::settings::Settings;
use crate::store::Store;
use crate::transport::Transport;
use crate::{Account, AchStatus, Event, GameProgress};

#[repr(C)]
pub(crate) struct Desc {
    pub flags: u64,
    pub ptr: *mut c_void,
    pub offset: usize,
    pub start: usize,
    pub select: usize,
    pub disconnect: usize,
    pub len: usize,
}

extern "C" {
    fn slot_rc_create(user_agent: *const c_char, verbose: c_int) -> c_int;
    fn slot_rc_login_password(user: *const c_char, password: *const c_char);
    fn slot_rc_login_token(user: *const c_char, token: *const c_char);
    fn slot_rc_logout();
    fn slot_rc_user(
        name: *mut c_char,
        name_size: usize,
        token: *mut c_char,
        token_size: usize,
    ) -> c_int;
    fn slot_rc_respond(
        callback: *mut c_void,
        data: *mut c_void,
        body: *const c_char,
        len: usize,
        status: c_int,
    );
    fn slot_rc_set_memory(
        descs: *const Desc,
        count: u32,
        console: u32,
        sysram: *mut u8,
        sysram_len: usize,
        saveram: *mut u8,
        saveram_len: usize,
    ) -> c_int;
    fn slot_rc_clear_memory();
    fn slot_rc_hash(console: u32, path: *const c_char, hash: *mut c_char) -> c_int;
    fn slot_rc_load_hash(hash: *const c_char);
    fn slot_rc_unload();
    fn slot_rc_frame();
    fn slot_rc_idle();
    fn slot_rc_reset();
    fn slot_rc_game(
        id: *mut u32,
        title: *mut c_char,
        title_size: usize,
        hash: *mut c_char,
        hash_size: usize,
        badge: *mut c_char,
        badge_size: usize,
        unlocked: *mut u32,
        total: *mut u32,
    ) -> c_int;
    fn slot_rc_each_badge();
    fn slot_rc_award_request(
        user: *const c_char,
        token: *const c_char,
        id: u32,
        hash: *const c_char,
        seconds: u32,
        url: *mut c_char,
        url_size: usize,
        post: *mut c_char,
        post_size: usize,
    ) -> c_int;
}

/// A request rcheevos is waiting on: its callback, and what to hand it.
struct Waiting {
    callback: usize,
    data: usize,
}

struct Request {
    url: String,
    body: String,
    content_type: String,
    waiting: Waiting,
}

struct Answered {
    waiting: Waiting,
    status: i32,
    body: CString,
}

enum Job {
    /// Hash the ROM, then load its achievements. Hashing reads the whole ROM, so it is done
    /// here rather than holding up the first frames of the game.
    Load {
        console: u32,
        path: CString,
    },
    Request(Request),
    Badges(Vec<String>),
    Flush,
}

/// What C callbacks recorded, for [`Core::settle`].
#[derive(Default)]
struct Raised {
    login: Option<(i32, String)>,
    load: Option<(i32, String)>,
    events: Vec<Event>,
    badges: Vec<String>,
    progress: bool,
}

pub(crate) struct Core {
    root: PathBuf,
    /// Held for every call into rcheevos.
    c: Mutex<()>,
    jobs: Mutex<Sender<Job>>,
    answered: Mutex<Vec<Answered>>,
    raised: Mutex<Raised>,
    status: Mutex<AchStatus>,
    events: Mutex<Vec<Event>>,
    settings: Mutex<Settings>,
    game_active: AtomicBool,
    online: fn() -> bool,
    /// A password login is waiting for its answer; on failure the old token stays.
    password_login: AtomicBool,
}

static CORE: OnceLock<Arc<Core>> = OnceLock::new();

/// How long after start a request waits for Wi-Fi before slot answers it from the card.
const BOOT_GRACE: Duration = Duration::from_secs(10);

pub(crate) fn core() -> Option<&'static Arc<Core>> {
    CORE.get()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn from_c(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: rcheevos hands us NUL-terminated strings that live for the call.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn buf_str(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Starts achievements for the card at `root`. Later calls do nothing.
pub(crate) fn start(root: &Path, transport: Box<dyn Transport>, online: fn() -> bool) {
    if CORE.get().is_some() {
        return;
    }
    let settings = Settings::load(root);
    let store = Store::new(root);
    let (tx, rx) = channel();
    let status = AchStatus {
        enabled: settings.enabled,
        account: match settings.token.is_empty() {
            true => Account::LoggedOut,
            false => Account::LoggingIn,
        },
        user: Some(settings.user.clone()).filter(|u| !u.is_empty()),
        pending: store.pending().len(),
        game: None,
        generation: 1,
    };
    let core = Arc::new(Core {
        root: root.to_path_buf(),
        c: Mutex::new(()),
        jobs: Mutex::new(tx),
        answered: Mutex::new(Vec::new()),
        raised: Mutex::new(Raised::default()),
        status: Mutex::new(status),
        events: Mutex::new(Vec::new()),
        settings: Mutex::new(settings.clone()),
        game_active: AtomicBool::new(false),
        online,
        password_login: AtomicBool::new(false),
    });
    if CORE.set(core.clone()).is_err() {
        return;
    }
    {
        let _c = lock(&core.c);
        let verbose = std::env::var_os("SLOT_ACH_VERBOSE").is_some() as c_int;
        // SAFETY: called once, under the lock, before anything else touches rcheevos.
        unsafe { slot_rc_create(std::ptr::null(), verbose) };
    }
    let proxy = Proxy::new(store, transport);
    let worker = core.clone();
    let _ = std::thread::Builder::new()
        .name("slot-ach".into())
        .spawn(move || worker.run(proxy, rx));
    if settings.enabled && !settings.token.is_empty() {
        core.login_with_token();
    }
}

impl Core {
    pub(crate) fn status(&self) -> AchStatus {
        lock(&self.status).clone()
    }

    fn update(&self, f: impl FnOnce(&mut AchStatus)) {
        let mut s = lock(&self.status);
        let before = s.clone();
        f(&mut s);
        if *s != before {
            s.generation += 1;
        }
    }

    pub(crate) fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.events))
    }

    pub(crate) fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    fn save_settings(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = lock(&self.settings);
        f(&mut s);
        if let Err(e) = s.save(&self.root) {
            eprintln!("slot-ach: {}: {e}", crate::settings::FILE);
        }
        let (enabled, user) = (s.enabled, s.user.clone());
        drop(s);
        self.update(|st| {
            st.enabled = enabled;
            st.user = Some(user).filter(|u| !u.is_empty());
        });
    }

    fn login_with_token(&self) {
        let s = self.settings();
        let _c = lock(&self.c);
        self.update(|st| st.account = Account::LoggingIn);
        let (u, t) = (cstr(&s.user), cstr(&s.token));
        // SAFETY: under the lock; rcheevos copies both strings.
        unsafe { slot_rc_login_token(u.as_ptr(), t.as_ptr()) };
    }

    pub(crate) fn set_enabled(&self, on: bool) {
        self.save_settings(|s| s.enabled = on);
        let s = self.settings();
        if on && !s.token.is_empty() && !matches!(self.status().account, Account::LoggedIn { .. }) {
            self.login_with_token();
        }
    }

    pub(crate) fn login(&self, user: &str, password: &str) {
        self.save_settings(|s| {
            s.enabled = true;
            s.user = user.to_string();
        });
        self.password_login.store(true, Ordering::Relaxed);
        let _c = lock(&self.c);
        self.update(|st| st.account = Account::LoggingIn);
        let (u, p) = (cstr(user), cstr(password));
        // SAFETY: under the lock; rcheevos copies both strings.
        unsafe {
            slot_rc_logout();
            slot_rc_login_password(u.as_ptr(), p.as_ptr());
        }
    }

    pub(crate) fn logout(&self) {
        {
            let _c = lock(&self.c);
            // SAFETY: under the lock.
            unsafe { slot_rc_logout() };
        }
        let user = self.settings().user;
        Store::new(&self.root).forget_login(&user);
        self.save_settings(|s| s.token.clear());
        self.update(|st| st.account = Account::LoggedOut);
    }

    pub(crate) fn send_now(&self) {
        let _ = lock(&self.jobs).send(Job::Flush);
    }

    /// Hands rcheevos the answers that have come back. Call with `c` held.
    fn pump(&self) {
        let answered = std::mem::take(&mut *lock(&self.answered));
        for a in answered {
            // SAFETY: the callback and its data are the ones rcheevos gave with the request,
            // each used exactly once; the body lives until the call returns.
            unsafe {
                slot_rc_respond(
                    a.waiting.callback as *mut c_void,
                    a.waiting.data as *mut c_void,
                    a.body.as_ptr(),
                    a.body.as_bytes().len(),
                    a.status,
                )
            };
        }
    }

    /// Acts on what C callbacks recorded, now that rcheevos has returned. Call with `c` held.
    fn settle(&self) {
        let raised = std::mem::take(&mut *lock(&self.raised));
        if let Some((result, error)) = raised.login {
            self.logged_in(result, &error);
        }
        let mut refresh = raised.progress;
        if let Some((result, error)) = raised.load {
            refresh = true;
            self.loaded(result, &error);
        }
        if refresh {
            let game = game_info();
            self.update(|st| st.game = game.or(st.game.take()));
        }
        if !raised.badges.is_empty() {
            let _ = lock(&self.jobs).send(Job::Badges(raised.badges));
        }
        if !raised.events.is_empty() {
            lock(&self.events).extend(raised.events);
        }
    }

    fn logged_in(&self, result: i32, error: &str) {
        let by_password = self.password_login.swap(false, Ordering::Relaxed);
        if result == 0 {
            let (mut name, mut token) = ([0u8; 128], [0u8; 128]);
            // SAFETY: under the lock; the buffers are as long as we say.
            let ok = unsafe {
                slot_rc_user(
                    name.as_mut_ptr().cast(),
                    name.len(),
                    token.as_mut_ptr().cast(),
                    token.len(),
                )
            } != 0;
            if ok {
                let (user, token) = (buf_str(&name), buf_str(&token));
                if !token.is_empty() {
                    self.save_settings(|s| {
                        s.user = user.clone();
                        s.token = token;
                    });
                }
                let offline = !(self.online)();
                self.update(|st| st.account = Account::LoggedIn { offline });
                lock(&self.events).push(Event::LoggedIn { user });
                return;
            }
        }
        let message = match error {
            "" => "Could not log in".to_string(),
            e => e.to_string(),
        };
        // A refused token is no use again; a password that failed leaves the old one be.
        if !by_password && (result == -34 || result == -35) {
            self.save_settings(|s| s.token.clear());
        }
        self.update(|st| st.account = Account::Failed(message.clone()));
        lock(&self.events).push(Event::LoginFailed(message));
    }

    fn loaded(&self, result: i32, error: &str) {
        if result == 0 {
            let game = game_info();
            let offline = !(self.online)();
            if let Some(g) = &game {
                lock(&self.events).push(Event::GameLoaded {
                    title: g.title.clone(),
                    unlocked: g.unlocked,
                    total: g.total,
                    badge: Store::new(&self.root).badge(&format!("game-{}", g.badge)),
                    offline,
                });
            }
            // SAFETY: under the lock; badge names come back through slot_rc_on_badge.
            unsafe { slot_rc_each_badge() };
            if let Some(g) = game.as_ref().filter(|g| !g.badge.is_empty()) {
                lock(&self.raised).badges.push(format!("game-{}", g.badge));
            }
            self.update(|st| st.game = game);
            return;
        }
        let event = match result {
            -29 => Event::NoAchievements,
            _ => Event::GameFailed(match error {
                "" => "Could not load achievements".into(),
                e => e.into(),
            }),
        };
        lock(&self.events).push(event);
        self.update(|st| st.game = None);
    }

    pub(crate) fn begin_game(&self, console: u32, rom: &Path, memory: &crate::Memory) -> bool {
        let s = self.settings();
        if !s.enabled
            || (s.token.is_empty() && !matches!(self.status().account, Account::LoggedIn { .. }))
        {
            return false;
        }
        let Some(path) = rom.to_str().map(cstr) else {
            return false;
        };
        let _c = lock(&self.c);
        self.game_active.store(true, Ordering::Release);
        // SAFETY: under the lock; the descriptors point into the running core, which outlives
        // the game (slot unloads the game before the core goes).
        unsafe {
            slot_rc_set_memory(
                memory.descs.as_ptr(),
                memory.descs.len() as u32,
                console,
                memory.system.0,
                memory.system.1,
                memory.save.0,
                memory.save.1,
            );
        }
        let _ = lock(&self.jobs).send(Job::Load { console, path });
        true
    }

    pub(crate) fn frame(&self) {
        let _c = lock(&self.c);
        self.pump();
        // SAFETY: under the lock, between two frames of the core whose memory rcheevos reads.
        unsafe { slot_rc_frame() };
        self.settle();
    }

    pub(crate) fn idle(&self) {
        let _c = lock(&self.c);
        self.pump();
        // SAFETY: under the lock.
        unsafe { slot_rc_idle() };
        self.settle();
    }

    pub(crate) fn reset(&self) {
        let _c = lock(&self.c);
        // SAFETY: under the lock.
        unsafe { slot_rc_reset() };
    }

    pub(crate) fn end_game(&self) {
        let _c = lock(&self.c);
        // SAFETY: under the lock; nothing reads the core's memory after this.
        unsafe {
            slot_rc_unload();
            slot_rc_clear_memory();
        }
        self.game_active.store(false, Ordering::Release);
        self.settle();
        self.update(|st| st.game = None);
    }

    fn run(self: Arc<Self>, proxy: Proxy, rx: Receiver<Job>) {
        let started = Instant::now();
        let mut flushed: Option<Instant> = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Job::Load { console, path }) => {
                    let mut hash = [0u8; 33];
                    // SAFETY: a path in, a 33-byte buffer out; no client state.
                    let ok =
                        unsafe { slot_rc_hash(console, path.as_ptr(), hash.as_mut_ptr().cast()) };
                    let _c = lock(&self.c);
                    if !self.game_active.load(Ordering::Acquire) {
                        continue;
                    }
                    if ok != 0 {
                        // SAFETY: under the lock; rcheevos copies the hash.
                        unsafe { slot_rc_load_hash(hash.as_ptr().cast()) };
                    } else {
                        lock(&self.raised).load = Some((-29, String::new()));
                        self.settle();
                    }
                }
                Ok(Job::Request(r)) => {
                    // Just after boot, give Wi-Fi a moment before answering from the card.
                    let mut online = (self.online)();
                    while !online && started.elapsed() < BOOT_GRACE {
                        std::thread::sleep(Duration::from_millis(250));
                        online = (self.online)();
                    }
                    let answer = proxy.serve(&r.url, &r.body, &r.content_type, online);
                    lock(&self.answered).push(Answered {
                        waiting: r.waiting,
                        status: answer.status,
                        body: cstr(&answer.body),
                    });
                    let pending = proxy.store.pending().len();
                    self.update(|st| st.pending = pending);
                }
                Ok(Job::Badges(names)) => self.fetch_badges(&proxy, &names),
                Ok(Job::Flush) => {
                    flushed = Some(Instant::now());
                    self.flush(&proxy);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if !self.game_active.load(Ordering::Acquire) {
                self.idle();
                // Logged in from the card while offline: log in properly once online.
                let stale = matches!(self.status().account, Account::LoggedIn { offline: true });
                if stale && (self.online)() {
                    self.login_with_token();
                }
            }
            let due = flushed.is_none_or(|t| t.elapsed() > Duration::from_secs(30));
            if due && (self.online)() {
                flushed = Some(Instant::now());
                self.flush(&proxy);
            }
        }
    }

    fn fetch_badges(&self, proxy: &Proxy, names: &[String]) {
        if !(self.online)() {
            return;
        }
        for name in names {
            let (url, key) = match name.strip_prefix("game-") {
                Some(game) => (
                    format!("https://media.retroachievements.org/Images/{game}.png"),
                    name.clone(),
                ),
                None => (
                    format!("https://media.retroachievements.org/Badge/{name}.png"),
                    name.clone(),
                ),
            };
            let dest = proxy.store.badge_path(&key);
            if !dest.is_file() {
                proxy.transport().fetch(&url, &dest);
            }
        }
    }

    /// Sends unlocks earned offline, oldest first.
    fn flush(&self, proxy: &Proxy) {
        let queue = proxy.store.pending();
        if queue.is_empty() {
            return;
        }
        let s = self.settings();
        if s.token.is_empty() {
            return;
        }
        let mut sent = 0;
        for q in &queue {
            let request = award_request(&s.user, &s.token, q.id, &q.hash, seconds_since(q.when));
            match proxy.send(q, request) {
                Sent::Accepted => sent += 1,
                Sent::Dropped(e) => eprintln!("slot-ach: unlock {} not accepted: {e}", q.id),
                Sent::Refused(e) => {
                    eprintln!("slot-ach: the server refused the login token: {e}");
                    lock(&self.events).push(Event::LoginFailed(
                        "Log in again to send saved unlocks".into(),
                    ));
                    break;
                }
                Sent::Unreachable => break,
            }
        }
        let pending = proxy.store.pending().len();
        self.update(|st| st.pending = pending);
        if sent > 0 {
            lock(&self.events).push(Event::Sent(sent));
        }
    }
}

fn game_info() -> Option<GameProgress> {
    let (mut id, mut unlocked, mut total) = (0u32, 0u32, 0u32);
    let (mut title, mut hash, mut badge) = ([0u8; 256], [0u8; 64], [0u8; 32]);
    // SAFETY: called with the lock held; the buffers are as long as we say.
    let ok = unsafe {
        slot_rc_game(
            &mut id,
            title.as_mut_ptr().cast(),
            title.len(),
            hash.as_mut_ptr().cast(),
            hash.len(),
            badge.as_mut_ptr().cast(),
            badge.len(),
            &mut unlocked,
            &mut total,
        )
    } != 0;
    ok.then(|| GameProgress {
        id,
        title: buf_str(&title),
        hash: buf_str(&hash),
        badge: buf_str(&badge),
        unlocked,
        total,
    })
}

fn award_request(
    user: &str,
    token: &str,
    id: u32,
    hash: &str,
    seconds: u32,
) -> Option<(String, String)> {
    let (u, t, h) = (cstr(user), cstr(token), cstr(hash));
    let (mut url, mut post) = (vec![0u8; 256], vec![0u8; 1024]);
    // SAFETY: plain strings in, buffers as long as we say; touches no rc_client state.
    let ok = unsafe {
        slot_rc_award_request(
            u.as_ptr(),
            t.as_ptr(),
            id,
            h.as_ptr(),
            seconds,
            url.as_mut_ptr().cast(),
            url.len(),
            post.as_mut_ptr().cast(),
            post.len(),
        )
    } != 0;
    ok.then(|| (buf_str(&url), buf_str(&post)))
}

// ---- Called from C ----------------------------------------------------------------------

#[no_mangle]
extern "C" fn slot_rc_on_request(
    url: *const c_char,
    post_data: *const c_char,
    content_type: *const c_char,
    callback: *mut c_void,
    callback_data: *mut c_void,
) {
    let Some(core) = core() else {
        return;
    };
    let request = Request {
        url: from_c(url),
        body: from_c(post_data),
        content_type: from_c(content_type),
        waiting: Waiting {
            callback: callback as usize,
            data: callback_data as usize,
        },
    };
    let _ = lock(&core.jobs).send(Job::Request(request));
}

#[no_mangle]
extern "C" fn slot_rc_on_done(kind: c_int, result: c_int, error: *const c_char) {
    let Some(core) = core() else {
        return;
    };
    let mut r = lock(&core.raised);
    match kind {
        1 => r.login = Some((result, from_c(error))),
        _ => r.load = Some((result, from_c(error))),
    }
}

#[no_mangle]
extern "C" fn slot_rc_on_event(
    kind: u32,
    _id: u32,
    title: *const c_char,
    description: *const c_char,
    badge: *const c_char,
    points: u32,
) {
    let Some(core) = core() else {
        return;
    };
    let event = match kind {
        1 => {
            let badge = from_c(badge);
            Event::Unlocked {
                title: from_c(title),
                description: from_c(description),
                points,
                badge: Store::new(&core.root).badge(&badge),
                later: !(core.online)(),
            }
        }
        15 => Event::Mastered,
        16 => Event::ServerError(format!("{}: {}", from_c(title), from_c(description))),
        _ => return,
    };
    let mut r = lock(&core.raised);
    r.progress = true;
    r.events.push(event);
}

#[no_mangle]
extern "C" fn slot_rc_on_badge(badge: *const c_char) {
    let Some(core) = core() else {
        return;
    };
    let name = from_c(badge);
    if !name.is_empty() {
        lock(&core.raised).badges.push(name);
    }
}

#[no_mangle]
extern "C" fn slot_rc_on_log(message: *const c_char) {
    eprintln!("slot-ach: {}", from_c(message));
}
