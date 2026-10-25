//! Saves and states laid out the way RetroArch keeps them, so they sync with RetroArch on other
//! devices: `Saves/mGBA/<game>.srm`, and `States/mGBA/<game>.state`, `.state1`, `.state2`… plus
//! `<game>.state.auto` for the resume state, each with a `.png` thumbnail beside it.
//!
//! Only GBA uses this layout; GB and GBC keep slot's own. The state data is the core's own
//! serialized state either way, which is what RetroArch stores. RetroArch may wrap it in a
//! `RASTATE` container and compress it with rzip (deflate or zstd); [`decode`] undoes both so
//! slot can load what RetroArch wrote. slot writes the bare core data, which RetroArch loads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::atomic::{atomic_write, sync_dir};
use crate::core::Core;
use crate::platform::Platform;
use crate::ring::{self, StateEntry};
use crate::stamp::{format_stamp, parse_stamp};

/// Where saves and states go.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Layout {
    /// slot's own: `Saves/GBA/<game>.sav`, `States/GBA/<core>/<game>/<stamp>.state`.
    Slot,
    /// RetroArch's, for GBA: `Saves/mGBA/<game>.srm`, `States/<core>/<game>.state<N>`.
    RetroArch,
}

static LAYOUT: AtomicU8 = AtomicU8::new(0);
static WALL_OFFSET_SECS: AtomicI64 = AtomicI64::new(0);

/// Chooses the layout for every save and state from here on. The library default is
/// [`Layout::Slot`]; slot's frontend sets it from the person's setting at boot.
pub fn set_layout(layout: Layout) {
    LAYOUT.store(layout as u8, Ordering::SeqCst);
}

pub fn layout() -> Layout {
    match LAYOUT.load(Ordering::SeqCst) {
        1 => Layout::RetroArch,
        _ => Layout::Slot,
    }
}

/// slot names states by wall-clock time; RetroArch's files only have their modified time,
/// which is UTC. The offset between them, so both sort together.
pub fn set_wall_offset_min(minutes: i16) {
    WALL_OFFSET_SECS.store(i64::from(minutes) * 60, Ordering::SeqCst);
}

fn wall_offset() -> i64 {
    WALL_OFFSET_SECS.load(Ordering::SeqCst)
}

/// RetroArch's per-core folder name: the core's `library_name`.
pub fn core_dir(core: Core) -> &'static str {
    match core {
        Core::Mgba => "mGBA",
        Core::Gpsp => "gpSP",
    }
}

/// The folder GBA battery saves live in under the RetroArch layout. One per game whichever
/// core slot runs it with, so switching cores never hides a save; mGBA's is what RetroArch
/// on other devices uses.
pub const SAVES_DIR: &str = "mGBA";

fn uses_retroarch(platform: Platform) -> bool {
    platform == Platform::Gba && layout() == Layout::RetroArch
}

/// Where a battery save is written: `Saves/mGBA/<game>.srm` for GBA under the RetroArch
/// layout, otherwise `None` and the caller keeps slot's own path.
pub fn save_path(root: &Path, platform: Platform, stem: &str) -> Option<PathBuf> {
    uses_retroarch(platform).then(|| {
        root.join("Saves")
            .join(SAVES_DIR)
            .join(format!("{stem}.srm"))
    })
}

// ------------------------------------------------------------------------------------------
// Decoding what RetroArch writes

const RZIP_MAGIC: &[u8; 6] = b"#RZIPv";
const RZIP_HEADER: usize = 20;
const RZIP_DEFLATE: u8 = 1;
const RZIP_ZSTD: u8 = 2;
/// RetroArch caps a chunk at 64 MB; so does slot, so a damaged header cannot ask for more.
const RZIP_MAX_CHUNK: usize = 64 * 1024 * 1024;
const RASTATE_MAGIC: &[u8; 7] = b"RASTATE";

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    BadChunk,
    UnknownCodec(u8),
    NoCoreData,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "file ends early"),
            DecodeError::BadChunk => write!(f, "compressed chunk would not inflate"),
            DecodeError::UnknownCodec(v) => write!(f, "rzip codec {v} is unknown"),
            DecodeError::NoCoreData => write!(f, "RASTATE file has no core block"),
        }
    }
}

impl std::error::Error for DecodeError {}

fn u32_le(b: &[u8]) -> usize {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize
}

/// The core data inside a file RetroArch wrote: rzip decompressed, then the `MEM ` block of a
/// `RASTATE` container taken out. Anything else, including slot's own files, comes back as is.
pub fn decode(bytes: Vec<u8>) -> Result<Vec<u8>, DecodeError> {
    let raw = unrzip(bytes)?;
    unrastate(raw)
}

fn unrzip(bytes: Vec<u8>) -> Result<Vec<u8>, DecodeError> {
    if bytes.len() < RZIP_HEADER || &bytes[..6] != RZIP_MAGIC || bytes[7] != b'#' {
        return Ok(bytes);
    }
    let codec = bytes[6];
    if codec != RZIP_DEFLATE && codec != RZIP_ZSTD {
        return Err(DecodeError::UnknownCodec(codec));
    }
    let chunk = u32_le(&bytes[8..12]);
    let total = u64::from_le_bytes(bytes[12..20].try_into().unwrap_or([0; 8])) as usize;
    if chunk == 0 || chunk > RZIP_MAX_CHUNK || total > 1 << 30 {
        return Err(DecodeError::BadChunk);
    }
    let mut out = Vec::with_capacity(total);
    let mut at = RZIP_HEADER;
    while out.len() < total {
        let head = bytes.get(at..at + 4).ok_or(DecodeError::Truncated)?;
        let len = u32_le(head);
        at += 4;
        let body = bytes.get(at..at + len).ok_or(DecodeError::Truncated)?;
        at += len;
        let piece = match codec {
            RZIP_DEFLATE => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, chunk)
                .map_err(|_| DecodeError::BadChunk)?,
            _ => {
                let mut src = body;
                let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src)
                    .map_err(|_| DecodeError::BadChunk)?;
                let mut v = Vec::with_capacity(chunk);
                std::io::Read::read_to_end(&mut dec, &mut v).map_err(|_| DecodeError::BadChunk)?;
                v
            }
        };
        if piece.is_empty() || piece.len() > chunk {
            return Err(DecodeError::BadChunk);
        }
        out.extend_from_slice(&piece);
    }
    out.truncate(total);
    Ok(out)
}

fn unrastate(bytes: Vec<u8>) -> Result<Vec<u8>, DecodeError> {
    if bytes.len() < 8 || &bytes[..7] != RASTATE_MAGIC {
        return Ok(bytes);
    }
    let mut at = 8;
    while at + 8 <= bytes.len() {
        let tag = &bytes[at..at + 4];
        let len = u32_le(&bytes[at + 4..at + 8]);
        at += 8;
        if tag == b"END " {
            break;
        }
        let body = bytes.get(at..at + len).ok_or(DecodeError::Truncated)?;
        if tag == b"MEM " {
            return Ok(body.to_vec());
        }
        at += (len + 7) & !7;
    }
    Err(DecodeError::NoCoreData)
}

/// Reads a file RetroArch or slot wrote and returns the core data in it.
pub fn read_decoded(path: &Path) -> std::io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    decode(bytes).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

// ------------------------------------------------------------------------------------------
// States

/// The state ring slot's App uses. Under the RetroArch layout a GBA game's ring is
/// RetroArch's numbered slots; everything else uses slot's own timestamped ring.
pub enum StateRing {
    Slot(ring::StateRing),
    RetroArch(RaRing),
}

impl StateRing {
    pub fn new(root: &Path, platform: Platform, core: Core, stem: &str) -> Self {
        match uses_retroarch(platform) {
            true => StateRing::RetroArch(RaRing::new(root, core, stem)),
            false => StateRing::Slot(ring::StateRing::new(root, platform, core, stem)),
        }
    }

    pub fn push(&self, state: &[u8], thumb_png: &[u8], stamp: &str) -> std::io::Result<()> {
        match self {
            StateRing::Slot(r) => r.push(state, thumb_png, stamp),
            StateRing::RetroArch(r) => r.push(state, thumb_png, stamp),
        }
    }

    pub fn list(&self) -> std::io::Result<Vec<StateEntry>> {
        match self {
            StateRing::Slot(r) => r.list(),
            StateRing::RetroArch(r) => r.list(),
        }
    }

    pub fn read(&self, stamp: &str) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
        match self {
            StateRing::Slot(r) => r.read(stamp),
            StateRing::RetroArch(r) => r.read(stamp),
        }
    }

    pub fn remove(&self, stamp: &str) -> std::io::Result<()> {
        match self {
            StateRing::Slot(r) => r.remove(stamp),
            StateRing::RetroArch(r) => r.remove(stamp),
        }
    }

    pub fn write_resume(&self, state: &[u8]) -> std::io::Result<()> {
        match self {
            StateRing::Slot(r) => r.write_resume(state),
            StateRing::RetroArch(r) => r.write_resume(state),
        }
    }

    pub fn read_resume(&self) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            StateRing::Slot(r) => r.read_resume(),
            StateRing::RetroArch(r) => r.read_resume(),
        }
    }

    pub fn retire_resume(&self, stamp: &str) -> std::io::Result<Option<PathBuf>> {
        match self {
            StateRing::Slot(r) => r.retire_resume(stamp),
            StateRing::RetroArch(r) => r.retire_resume(stamp),
        }
    }
}

/// One game's states in RetroArch's layout.
///
/// The list shows `<game>.state.auto` first, then the numbered slots from the highest number
/// down, however many there are. A new save goes one above the highest slot, the way
/// RetroArch's auto-increment numbers them, so nothing is overwritten. The resume state slot
/// loads on insert is whichever file is newest.
///
/// slot knows each state by a wall-clock stamp. RetroArch's files carry no stamp, so one is
/// read from the file's modified time. FAT32 keeps modified times to 2 seconds, so the stamps
/// slot gave its own saves are also kept in a small index under `States/.slot`, which is not
/// synced; an entry is only believed while the file still has the length and time it had.
pub struct RaRing {
    dir: PathBuf,
    private: PathBuf,
    stem: String,
}

/// A state file on the card: `None` for `.state.auto`, else its slot number.
#[derive(Clone, Debug)]
struct Found {
    slot: Option<u32>,
    path: PathBuf,
    mtime: i64,
    len: u64,
}

/// What slot remembers of a slot it wrote itself.
#[derive(Clone, Debug, PartialEq)]
struct Known {
    stamp: String,
    mtime: i64,
    len: u64,
}

pub const PRIVATE_DIR: &str = ".slot";
const AUTO: &str = "auto";

fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `.state.auto` first, then the highest slot number down.
fn display_order(a: &Found, b: &Found) -> std::cmp::Ordering {
    match (a.slot, b.slot) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => y.cmp(&x),
    }
}

impl RaRing {
    pub fn new(root: &Path, core: Core, stem: &str) -> Self {
        let states = root.join("States");
        RaRing {
            dir: states.join(core_dir(core)),
            private: states.join(PRIVATE_DIR).join(core_dir(core)),
            stem: stem.to_string(),
        }
    }

    /// `<game>.state` for slot 0, `<game>.state<N>` after.
    pub fn slot_path(&self, slot: u32) -> PathBuf {
        match slot {
            0 => self.dir.join(format!("{}.state", self.stem)),
            n => self.dir.join(format!("{}.state{n}", self.stem)),
        }
    }

    pub fn auto_path(&self) -> PathBuf {
        self.dir.join(format!("{}.state.{AUTO}", self.stem))
    }

    fn thumb_of(path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(".png");
        PathBuf::from(s)
    }

    fn index_path(&self) -> PathBuf {
        self.private.join(format!("{}.stamps", self.stem))
    }

    fn found(&self) -> std::io::Result<Vec<Found>> {
        let dir = match std::fs::read_dir(&self.dir) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let prefix = format!("{}.state", self.stem);
        let mut out = Vec::new();
        for entry in dir {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let slot = match rest {
                "" => Some(0),
                ".auto" => None,
                n if (1..=9).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()) => {
                    match n.parse() {
                        Ok(v) => Some(v),
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };
            let meta = entry.metadata()?;
            if !meta.is_file() || meta.len() == 0 {
                continue;
            }
            out.push(Found {
                slot,
                path: entry.path(),
                mtime: meta.modified().map(unix_secs).unwrap_or(0),
                len: meta.len(),
            });
        }
        Ok(out)
    }

    fn read_index(&self) -> HashMap<u32, Known> {
        let text = std::fs::read_to_string(self.index_path()).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let mut f = l.split('\t');
                let slot = f.next()?.parse().ok()?;
                let stamp = f.next()?.to_string();
                let mtime = f.next()?.parse().ok()?;
                let len = f.next()?.parse().ok()?;
                parse_stamp(&stamp)?;
                Some((slot, Known { stamp, mtime, len }))
            })
            .collect()
    }

    fn write_index(&self, index: &HashMap<u32, Known>) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.private)?;
        let mut slots: Vec<_> = index.iter().collect();
        slots.sort_by_key(|(s, _)| **s);
        let text: String = slots
            .into_iter()
            .map(|(s, k)| format!("{s}\t{}\t{}\t{}\n", k.stamp, k.mtime, k.len))
            .collect();
        atomic_write(&self.index_path(), text.as_bytes())
    }

    /// Every state with a stamp unique to it, oldest first. Stamps follow time: a later file
    /// never shares one with an earlier.
    fn by_time(&self) -> std::io::Result<Vec<(Found, String)>> {
        let index = self.read_index();
        let found = self.found()?;
        let secs = |f: &Found| -> i64 {
            let known = f.slot.and_then(|s| index.get(&s));
            match known {
                // FAT32 rounds modified times to 2 s; allow for that and nothing more.
                Some(k) if k.len == f.len && (k.mtime - f.mtime).abs() <= 2 => {
                    parse_stamp(&k.stamp).unwrap_or(f.mtime + wall_offset())
                }
                _ => f.mtime + wall_offset(),
            }
        };
        let mut timed: Vec<(i64, Found)> = found.into_iter().map(|f| (secs(&f), f)).collect();
        timed.sort_by(|a, b| a.0.cmp(&b.0).then(display_order(&b.1, &a.1)));
        let mut out = Vec::with_capacity(timed.len());
        let mut last: Option<i64> = None;
        for (mut t, f) in timed {
            if let Some(prev) = last {
                t = t.max(prev + 1);
            }
            last = Some(t);
            out.push((f, format_stamp(t)));
        }
        Ok(out)
    }

    fn find(&self, stamp: &str) -> std::io::Result<Found> {
        self.by_time()?
            .into_iter()
            .find(|(_, s)| s == stamp)
            .map(|(f, _)| f)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no state stamped {stamp}"),
                )
            })
    }

    /// `.state.auto` first, then numbered slots from the highest down.
    pub fn list(&self) -> std::io::Result<Vec<StateEntry>> {
        let mut all = self.by_time()?;
        all.sort_by(|a, b| display_order(&a.0, &b.0));
        Ok(all
            .into_iter()
            .map(|(f, stamp)| StateEntry {
                thumb: Self::thumb_of(&f.path),
                state: f.path,
                stamp,
            })
            .collect())
    }

    /// The slot a new save goes in: one above the highest, or `.state` when there are none.
    pub fn next_slot(&self) -> std::io::Result<u32> {
        let highest = self.found()?.into_iter().filter_map(|f| f.slot).max();
        Ok(highest.map_or(0, |h| h.saturating_add(1)))
    }

    pub fn push(&self, state: &[u8], thumb_png: &[u8], stamp: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let slot = self.next_slot()?;
        let path = self.slot_path(slot);
        let thumb = Self::thumb_of(&path);
        if thumb_png.is_empty() {
            let _ = std::fs::remove_file(&thumb);
        } else {
            atomic_write(&thumb, thumb_png)?;
        }
        atomic_write(&path, state)?;
        let utc = parse_stamp(stamp).map(|s| s - wall_offset());
        if let Some(utc) = utc.filter(|s| *s > 0) {
            let at = UNIX_EPOCH + Duration::from_secs(utc as u64);
            if let Ok(f) = std::fs::File::options().write(true).open(&path) {
                let _ = f.set_modified(at);
            }
        }
        let meta = std::fs::metadata(&path)?;
        let mut index = self.read_index();
        index.insert(
            slot,
            Known {
                stamp: stamp.to_string(),
                mtime: meta.modified().map(unix_secs).unwrap_or(0),
                len: meta.len(),
            },
        );
        self.write_index(&index)
    }

    pub fn read(&self, stamp: &str) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
        let f = self.find(stamp)?;
        let state = read_decoded(&f.path)?;
        let thumb = std::fs::read(Self::thumb_of(&f.path)).unwrap_or_default();
        Ok((state, thumb))
    }

    pub fn remove(&self, stamp: &str) -> std::io::Result<()> {
        let f = self.find(stamp)?;
        std::fs::remove_file(&f.path)?;
        let _ = std::fs::remove_file(Self::thumb_of(&f.path));
        if let Some(slot) = f.slot {
            let mut index = self.read_index();
            if index.remove(&slot).is_some() {
                self.write_index(&index)?;
            }
        }
        Ok(())
    }

    pub fn write_resume(&self, state: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(&self.auto_path(), state)
    }

    /// The newest state of all, `.state.auto` or a numbered slot, whichever device wrote it.
    fn newest(&self) -> std::io::Result<Option<Found>> {
        Ok(self.by_time()?.pop().map(|(f, _)| f))
    }

    /// What slot loads on insert: the newest state, so play picks up where any device left off.
    pub fn read_resume(&self) -> std::io::Result<Option<Vec<u8>>> {
        match self.newest()? {
            Some(f) => read_decoded(&f.path).map(Some),
            None => Ok(None),
        }
    }

    /// Moves the state the core refused, the one [`Self::read_resume`] gave, out of RetroArch's
    /// folder into `States/.slot`, so neither slot nor RetroArch tries it again; it is kept for
    /// a look later. The next start falls back to the next newest.
    pub fn retire_resume(&self, stamp: &str) -> std::io::Result<Option<PathBuf>> {
        let Some(from) = self.newest()?.map(|f| f.path) else {
            return Ok(None);
        };
        std::fs::create_dir_all(&self.private)?;
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let base = format!("{name}.refused-{stamp}");
        let to = (1u32..)
            .map(|n| match n {
                1 => self.private.join(&base),
                n => self.private.join(format!("{base}-{n}")),
            })
            .find(|p| !p.exists())
            .unwrap_or_else(|| self.private.join(&base));
        std::fs::rename(&from, &to)?;
        let _ = std::fs::remove_file(Self::thumb_of(&from));
        sync_dir(&to);
        Ok(Some(to))
    }
}
