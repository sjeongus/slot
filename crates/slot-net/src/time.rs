//! Time zones: which one this device is in, and its UTC offset at any moment, daylight
//! saving included.
//!
//! The zone is looked up once online from the public IP address and remembered. Offsets come
//! from the zone's rules in the system time zone database (`/usr/share/zoneinfo`, which BaseOS
//! ships), so clocks change for daylight saving on their own, offline too.

use std::path::{Path, PathBuf};

pub const ZONEINFO: &str = "/usr/share/zoneinfo";

fn zoneinfo_dir() -> PathBuf {
    std::env::var_os("SLOT_ZONEINFO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(ZONEINFO))
}

/// An IANA zone name that names a file inside the database and nothing outside it.
pub fn valid_zone(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() < 64
        && !zone.starts_with('/')
        && !zone
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-+".contains(&b))
}

fn zone_file(zone: &str) -> Option<PathBuf> {
    valid_zone(zone)
        .then(|| zoneinfo_dir().join(zone))
        .filter(|p| p.is_file())
}

/// `zone`'s offset from UTC, in seconds, at `utc` (Unix seconds).
pub fn zone_offset_secs(zone: &str, utc: i64) -> Option<i32> {
    let bytes = std::fs::read(zone_file(zone)?).ok()?;
    Tzif::parse(&bytes)?.offset_at(utc)
}

/// The short city name for a zone: `America/Los_Angeles` reads as `Los Angeles`.
pub fn city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

// ------------------------------------------------------------------------------------------
// TZif, RFC 8536

struct Tzif {
    transitions: Vec<i64>,
    kinds: Vec<u8>,
    offsets: Vec<(i32, bool)>,
    footer: Option<PosixTz>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl Tzif {
    fn parse(b: &[u8]) -> Option<Tzif> {
        let counts = Self::counts(b)?;
        if b[4] == 0 {
            return Self::body(b.get(44..)?, counts, 4, None);
        }
        // Version 2 and later repeat the data with 64-bit times, then a POSIX TZ footer.
        let second = b.get(44 + Self::data_len(counts, 4)..)?;
        let counts2 = Self::counts(second)?;
        let data = second.get(44..)?;
        let footer = data
            .get(Self::data_len(counts2, 8)..)
            .and_then(|f| std::str::from_utf8(f).ok())
            .and_then(|f| f.trim_matches('\n').lines().next().map(str::to_string))
            .and_then(|f| PosixTz::parse(&f));
        Self::body(data, counts2, 8, footer)
    }

    /// isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt.
    fn counts(b: &[u8]) -> Option<[usize; 6]> {
        if b.get(..4)? != b"TZif" || b.len() < 44 {
            return None;
        }
        let mut c = [0usize; 6];
        for (i, v) in c.iter_mut().enumerate() {
            *v = be32(&b[20 + i * 4..]) as usize;
        }
        Some(c)
    }

    fn data_len(c: [usize; 6], tsize: usize) -> usize {
        let [isut, isstd, leap, time, kind, chars] = c;
        time * tsize + time + kind * 6 + chars + leap * (tsize + 4) + isstd + isut
    }

    fn body(d: &[u8], c: [usize; 6], tsize: usize, footer: Option<PosixTz>) -> Option<Tzif> {
        let [_, _, _, time, kind, _] = c;
        let mut at = 0;
        let mut transitions = Vec::with_capacity(time);
        for _ in 0..time {
            let t = match tsize {
                4 => i64::from(be32(d.get(at..at + 4)?) as i32),
                _ => i64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?),
            };
            transitions.push(t);
            at += tsize;
        }
        let kinds = d.get(at..at + time)?.to_vec();
        at += time;
        let mut offsets = Vec::with_capacity(kind);
        for _ in 0..kind {
            let e = d.get(at..at + 6)?;
            offsets.push((be32(e) as i32, e[4] != 0));
            at += 6;
        }
        if offsets.is_empty() || kinds.iter().any(|k| *k as usize >= offsets.len()) {
            return None;
        }
        Some(Tzif {
            transitions,
            kinds,
            offsets,
            footer,
        })
    }

    fn offset_at(&self, utc: i64) -> Option<i32> {
        let after_last = self.transitions.last().is_none_or(|last| utc >= *last);
        if after_last {
            if let Some(f) = &self.footer {
                return Some(f.offset_at(utc));
            }
        }
        match self.transitions.iter().rposition(|t| *t <= utc) {
            Some(i) => Some(self.offsets[self.kinds[i] as usize].0),
            // Before the first transition: the first standard time.
            None => Some(
                self.offsets
                    .iter()
                    .find(|(_, dst)| !dst)
                    .unwrap_or(&self.offsets[0])
                    .0,
            ),
        }
    }
}

// ------------------------------------------------------------------------------------------
// POSIX TZ strings, the rule a zone keeps following after its last listed transition

#[derive(Clone, Debug, PartialEq)]
enum Rule {
    /// Mm.w.d: day d (0 = Sunday) of week w (5 = last) of month m.
    Month { m: u32, w: u32, d: u32 },
    /// Jn: day n of the year, 1 to 365, February 29 never counted.
    Julian1(u32),
    /// n: day n of the year, 0 to 365, February 29 counted.
    Julian0(u32),
}

#[derive(Clone, Debug, PartialEq)]
struct PosixTz {
    /// Seconds east of UTC, as TZif counts them (POSIX writes them west).
    std: i32,
    dst: Option<(i32, Rule, i32, Rule, i32)>,
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn name(&mut self) -> Option<()> {
        if self.eat(b'<') {
            while self.peek()? != b'>' {
                self.i += 1;
            }
            self.i += 1;
            return Some(());
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        (self.i - start >= 3).then_some(())
    }

    fn number(&mut self) -> Option<i32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }

    /// [+-]hh[:mm[:ss]] as seconds.
    fn time(&mut self) -> Option<i32> {
        let sign = if self.eat(b'-') {
            -1
        } else {
            self.eat(b'+');
            1
        };
        let mut secs = self.number()? * 3600;
        if self.eat(b':') {
            secs += self.number()? * 60;
            if self.eat(b':') {
                secs += self.number()?;
            }
        }
        Some(sign * secs)
    }

    fn rule(&mut self) -> Option<(Rule, i32)> {
        let rule = if self.eat(b'M') {
            let m = self.number()? as u32;
            self.eat(b'.').then_some(())?;
            let w = self.number()? as u32;
            self.eat(b'.').then_some(())?;
            let d = self.number()? as u32;
            ((1..=12).contains(&m) && (1..=5).contains(&w) && d <= 6).then_some(())?;
            Rule::Month { m, w, d }
        } else if self.eat(b'J') {
            Rule::Julian1(self.number()? as u32)
        } else {
            Rule::Julian0(self.number()? as u32)
        };
        let at = if self.eat(b'/') { self.time()? } else { 7200 };
        Some((rule, at))
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn year_of(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + (m <= 2) as i64
}

fn month_len(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The day (days since 1970-01-01) a rule falls on in year `y`.
fn rule_day(rule: &Rule, y: i64) -> i64 {
    match *rule {
        Rule::Month { m, w, d } => {
            let (m, w, d) = (i64::from(m), i64::from(w), i64::from(d));
            let first = days_from_civil(y, m, 1);
            let first_dow = (first + 4).rem_euclid(7);
            let mut day = first + (d - first_dow).rem_euclid(7) + (w - 1) * 7;
            let last = first + month_len(y, m) - 1;
            while day > last {
                day -= 7;
            }
            day
        }
        Rule::Julian1(n) => {
            let n = i64::from(n);
            days_from_civil(y, 1, 1) + n - 1 + (is_leap(y) && n > 59) as i64
        }
        Rule::Julian0(n) => days_from_civil(y, 1, 1) + i64::from(n),
    }
}

impl PosixTz {
    fn parse(s: &str) -> Option<PosixTz> {
        let mut c = Cursor {
            s: s.as_bytes(),
            i: 0,
        };
        c.name()?;
        let std = -c.time()?;
        if c.peek().is_none() {
            return Some(PosixTz { std, dst: None });
        }
        c.name()?;
        let dst_off = match c.peek() {
            Some(b',') | None => std + 3600,
            _ => -c.time()?,
        };
        if !c.eat(b',') {
            return Some(PosixTz { std, dst: None });
        }
        let (start, start_at) = c.rule()?;
        c.eat(b',').then_some(())?;
        let (end, end_at) = c.rule()?;
        Some(PosixTz {
            std,
            dst: Some((dst_off, start, start_at, end, end_at)),
        })
    }

    fn offset_at(&self, utc: i64) -> i32 {
        let Some((dst, start, start_at, end, end_at)) = &self.dst else {
            return self.std;
        };
        let year = year_of((utc + i64::from(self.std)).div_euclid(86_400));
        // Rule times are local: the start in standard time, the end in daylight time.
        let begins = rule_day(start, year) * 86_400 + i64::from(*start_at) - i64::from(self.std);
        let ends = rule_day(end, year) * 86_400 + i64::from(*end_at) - i64::from(*dst);
        let in_dst = if begins < ends {
            utc >= begins && utc < ends
        } else {
            // Southern hemisphere: daylight time spans the new year.
            !(utc >= ends && utc < begins)
        };
        if in_dst {
            *dst
        } else {
            self.std
        }
    }
}

/// Reads a zone from the database and checks it parses; for tests and diagnostics.
pub fn zone_readable(zone: &str) -> bool {
    zone_file(zone)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| Tzif::parse(&b))
        .is_some()
}

/// Asks a public service which zone this connection's IP address is in. ip-api.com first,
/// worldtimeapi.org if it does not answer. Both see the public IP address, nothing else.
pub fn lookup_zone() -> Option<String> {
    let tries: [(&str, &str, &str); 2] = [
        ("ip-api.com", "/json/?fields=status,timezone", "timezone"),
        ("worldtimeapi.org", "/api/ip", "timezone"),
    ];
    for (host, path, key) in tries {
        let Some(r) = crate::http::get(host, 80, path, std::time::Duration::from_secs(6)) else {
            continue;
        };
        let zone = r
            .json()
            .and_then(|j| j.get(key).as_str().map(str::to_string))
            .filter(|z| valid_zone(z));
        if zone.is_some() {
            return zone;
        }
    }
    None
}

pub fn zoneinfo_has(dir: &Path, zone: &str) -> bool {
    valid_zone(zone) && dir.join(zone).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_names_cannot_escape_the_database() {
        assert!(valid_zone("America/Los_Angeles"));
        assert!(valid_zone("Etc/GMT+8"));
        assert!(!valid_zone("../etc/passwd"));
        assert!(!valid_zone("/etc/passwd"));
        assert!(!valid_zone("America//X"));
        assert!(!valid_zone(""));
        assert_eq!(city("America/Los_Angeles"), "Los Angeles");
    }

    #[test]
    fn posix_rules_follow_us_and_european_and_southern_daylight_time() {
        let la = PosixTz::parse("PST8PDT,M3.2.0,M11.1.0").unwrap();
        // 2026-03-08 09:59:59 UTC is 01:59:59 PST; a second later it is 03:00 PDT.
        assert_eq!(la.offset_at(1_772_963_999), -8 * 3600);
        assert_eq!(la.offset_at(1_772_964_000), -7 * 3600);
        // 2026-11-01 08:59:59 UTC is 01:59:59 PDT; a second later, 01:00 PST again.
        assert_eq!(la.offset_at(1_793_523_599), -7 * 3600);
        assert_eq!(la.offset_at(1_793_523_600), -8 * 3600);
        let berlin = PosixTz::parse("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(berlin.offset_at(1_774_745_999), 3600);
        assert_eq!(berlin.offset_at(1_774_746_000), 7200);
        let sydney = PosixTz::parse("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap();
        assert_eq!(sydney.offset_at(1_767_225_600), 11 * 3600);
        assert_eq!(sydney.offset_at(1_782_864_000), 10 * 3600);
        let tokyo = PosixTz::parse("JST-9").unwrap();
        assert_eq!(tokyo.offset_at(0), 9 * 3600);
        let india = PosixTz::parse("IST-5:30").unwrap();
        assert_eq!(india.offset_at(0), 5 * 3600 + 1800);
        let numeric = PosixTz::parse("<-03>3").unwrap();
        assert_eq!(numeric.offset_at(0), -3 * 3600);
    }

    #[test]
    fn the_system_database_matches_known_offsets_when_present() {
        if !zone_readable("America/Los_Angeles") {
            return;
        }
        assert_eq!(
            zone_offset_secs("America/Los_Angeles", 1_793_000_000),
            Some(-7 * 3600)
        );
        assert_eq!(
            zone_offset_secs("America/Los_Angeles", 1_800_000_000),
            Some(-8 * 3600)
        );
        assert_eq!(
            zone_offset_secs("Asia/Kolkata", 1_800_000_000),
            Some(19_800)
        );
        assert_eq!(
            zone_offset_secs("Australia/Sydney", 1_767_225_600),
            Some(39_600)
        );
        // Far past the table: only the footer rule can answer.
        assert_eq!(zone_offset_secs("Europe/Berlin", 4_102_444_800), Some(3600));
        assert_eq!(zone_offset_secs("Nowhere/Atlantis", 0), None);
    }
}

#[cfg(test)]
mod crosscheck {
    /// Compares with offsets another implementation computed, when given a file of
    /// `zone utc offset` lines in SLOT_TZ_CASES.
    #[test]
    fn matches_a_reference_implementation() {
        let Some(path) = std::env::var_os("SLOT_TZ_CASES") else {
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let mut bad = Vec::new();
        for line in text.lines() {
            let mut f = line.split(' ');
            let (zone, utc, want) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
            let got = super::zone_offset_secs(zone, utc.parse().unwrap());
            if got != Some(want.parse().unwrap()) {
                bad.push(format!("{zone} {utc}: got {got:?}, want {want}"));
            }
        }
        assert!(
            bad.is_empty(),
            "{} mismatches, e.g. {:?}",
            bad.len(),
            &bad[..bad.len().min(8)]
        );
    }
}

// ------------------------------------------------------------------------------------------
// The system clock's settings

/// What slot asks of the system for time: BaseOS's `timedatectl` on a device, nothing on the
/// host.
pub trait Clock: Send {
    /// Turns BaseOS's NTP service on or off. It waits for a network on its own.
    fn set_ntp(&mut self, on: bool);
    /// Tells BaseOS the zone, so its local time agrees with slot's.
    fn set_timezone(&mut self, zone: &str);
    /// The zone this connection's public IP address is in.
    fn lookup_zone(&mut self) -> Option<String>;
    /// NTP has set the clock this boot.
    fn ntp_synced(&mut self) -> bool;
}

/// BaseOS: `timedatectl set-ntp` and `set-timezone`, which persist on `/data`.
pub struct BaseosClock {
    tool: PathBuf,
}

impl BaseosClock {
    pub fn new() -> BaseosClock {
        BaseosClock {
            tool: PathBuf::from("/usr/bin/timedatectl"),
        }
    }

    fn run(&self, args: &[&str]) -> Option<String> {
        if !self.tool.exists() {
            return None;
        }
        let out = std::process::Command::new(&self.tool)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

impl Default for BaseosClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for BaseosClock {
    fn set_ntp(&mut self, on: bool) {
        let want = if on { "yes" } else { "no" };
        if self.run(&["show", "-p", "NTP", "--value"]).as_deref() != Some(want) {
            self.run(&["set-ntp", if on { "true" } else { "false" }]);
        }
    }

    fn set_timezone(&mut self, zone: &str) {
        if !valid_zone(zone) {
            return;
        }
        if self.run(&["show", "-p", "Timezone", "--value"]).as_deref() != Some(zone) {
            self.run(&["set-timezone", zone]);
        }
    }

    fn lookup_zone(&mut self) -> Option<String> {
        lookup_zone().filter(|z| zone_readable(z))
    }

    fn ntp_synced(&mut self) -> bool {
        self.run(&["show", "-p", "NTPSynchronized", "--value"])
            .as_deref()
            == Some("yes")
    }
}

/// The host build's clock: remembers what it was told and "finds" Los Angeles.
#[derive(Default)]
pub struct SimClock {
    pub ntp: bool,
    pub zone: Option<String>,
    pub lookups: usize,
}

impl Clock for SimClock {
    fn set_ntp(&mut self, on: bool) {
        self.ntp = on;
    }

    fn set_timezone(&mut self, zone: &str) {
        self.zone = Some(zone.to_string());
    }

    fn lookup_zone(&mut self) -> Option<String> {
        self.lookups += 1;
        Some("America/Los_Angeles".into())
    }

    fn ntp_synced(&mut self) -> bool {
        self.ntp
    }
}
