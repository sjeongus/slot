//! What slot's own network features (Syncthing, Tailscale, RetroAchievements) need from the
//! UI, drawn the way upstream's quick menu and pairing screen draw theirs: status text for
//! their rows, and an info screen with a QR code for a device ID or a login link.
//!
//! Faces are built on demand: a draw asks [`PageFaces`] for what it wants, and the frontend
//! rasterises whatever was missing before the next frame.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use slot_net::{NetStatus, SyncState, TailState};

use crate::draw::{Canvas, Draw, TexId};
use crate::network_menu::{pair_text_width, PAIR_GAP, PAIR_QR, PAIR_TOP, PAIR_X};
use crate::plate::UndoFace;
use crate::power_menu::MENU_INK;
use crate::quick_menu::{draw_menu_ground, DIM_INK};
use crate::text;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PageFace {
    /// A quick menu value, exactly as the quick menu draws its own.
    Label(String, bool),
    /// Lines of an info screen and the notice: values lit, labels dim.
    Value(String, bool),
    Small(String),
    /// Device IDs, addresses and passwords.
    Mono(String),
    Qr(String),
    /// An achievement badge from the card.
    Badge(std::path::PathBuf),
}

const DIM: [u8; 3] = DIM_INK;
const VALUE_PX: f32 = 24.0;
const SMALL_PX: f32 = 20.0;
const MONO_PX: f32 = 20.0;
const MAX_FACE_W: u32 = 600;
const QR_SCALE: u32 = 4;
const QR_SIDE: u32 = PAIR_QR;

fn empty() -> UndoFace {
    UndoFace {
        rgba: Vec::new(),
        w: 0,
        h: 0,
    }
}

/// One line of text in its own case, cut short with an ellipsis past `max_w`.
pub fn plain_face(line: &str, px: f32, ink: [u8; 3], max_w: u32) -> UndoFace {
    let Some(font) = text::label_font() else {
        return empty();
    };
    let Some(vm) = font.horizontal_line_metrics(px) else {
        return empty();
    };
    let width = |s: &str| text::line_width(font, s, px, 0.0);
    let mut shown: String = line.to_string();
    if width(&shown) > max_w as f32 {
        while !shown.is_empty() && width(&format!("{shown}…")) > max_w as f32 {
            shown.pop();
        }
        shown.push('…');
    }
    let w = (width(&shown).ceil() as u32 + 2).max(1);
    let h = vm.new_line_size.ceil() as u32 + 2;
    let mut cov = vec![0u8; (w * h) as usize];
    let baseline = 1.0 + vm.ascent;
    let mut pen = 1.0f32;
    for ch in shown.chars() {
        let (m, glyph) = font.rasterize(ch, px);
        let x0 = (pen + m.xmin as f32).round() as i32;
        let y0 = (baseline - (m.height as f32 + m.ymin as f32)).round() as i32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let (x, y) = (x0 + gx as i32, y0 + gy as i32);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let at = (y as u32 * w + x as u32) as usize;
                cov[at] = cov[at].max(glyph[gy * m.width + gx]);
            }
        }
        pen += m.advance_width;
    }
    tint(&cov, w, h, ink)
}

fn tint(cov: &[u8], w: u32, h: u32, ink: [u8; 3]) -> UndoFace {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (i, a) in cov.iter().enumerate() {
        rgba[i * 4..i * 4 + 4].copy_from_slice(&[ink[0], ink[1], ink[2], *a]);
    }
    UndoFace { rgba, w, h }
}

/// The QR code of `data`, black on white with a quiet zone, ready to scan from a phone.
pub fn qr_face(data: &str) -> UndoFace {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M) else {
        return empty();
    };
    let n = code.width() as u32;
    let quiet = 4;
    // Bigger when the data is short, so a short link scans from further away.
    let scale = (QR_SIDE / (n + 2 * quiet)).clamp(QR_SCALE, 8);
    let side = (n + 2 * quiet) * scale;
    let mut rgba = vec![0xffu8; (side * side * 4) as usize];
    let colors = code.to_colors();
    for my in 0..n {
        for mx in 0..n {
            if colors[(my * n + mx) as usize] != qrcode::Color::Dark {
                continue;
            }
            for py in 0..scale {
                for px in 0..scale {
                    let x = (mx + quiet) * scale + px;
                    let y = (my + quiet) * scale + py;
                    let at = ((y * side + x) * 4) as usize;
                    rgba[at..at + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    UndoFace {
        rgba,
        w: side,
        h: side,
    }
}

pub fn page_face(face: &PageFace) -> UndoFace {
    let ink = |lit: bool| if lit { MENU_INK } else { DIM };
    match face {
        PageFace::Label(t, lit) => crate::quick_menu::quick_value_face(t, *lit),
        PageFace::Value(t, lit) => plain_face(t, VALUE_PX, ink(*lit), MAX_FACE_W),
        PageFace::Small(t) => plain_face(t, SMALL_PX, DIM, MAX_FACE_W),
        PageFace::Mono(t) => plain_face(t, MONO_PX, MENU_INK, MAX_FACE_W),
        PageFace::Qr(data) => qr_face(data),
        PageFace::Badge(path) => crate::notice::achievement_badge_face(path),
    }
}

/// Textures for faces, filled in by the frontend. Draw notes what it wanted, so the frontend
/// knows what to build and what it may recycle.
#[derive(Default)]
pub struct PageFaces {
    pub ready: HashMap<PageFace, (TexId, u32, u32)>,
    wanted: RefCell<Vec<PageFace>>,
    used: RefCell<HashSet<PageFace>>,
}

impl PageFaces {
    pub fn get(&self, face: PageFace) -> Option<(TexId, u32, u32)> {
        match self.ready.get(&face) {
            Some(t) => {
                self.used.borrow_mut().insert(face);
                Some(*t)
            }
            None => {
                let mut w = self.wanted.borrow_mut();
                if !w.contains(&face) {
                    w.push(face);
                }
                None
            }
        }
    }

    /// Faces the last draw asked for and did not have.
    pub fn take_wanted(&self) -> Vec<PageFace> {
        std::mem::take(&mut self.wanted.borrow_mut())
    }

    /// Faces the last draw used. Everything else can be recycled.
    pub fn take_used(&self) -> HashSet<PageFace> {
        std::mem::take(&mut self.used.borrow_mut())
    }
}

/// What the Achievements rows show, filled in by slot from its achievements status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AchView {
    pub enabled: bool,
    pub user: Option<String>,
    pub logged_in: bool,
    /// "Logged in", "Logging in…", or why it failed.
    pub state: String,
    /// Unlocks saved offline, waiting to be sent.
    pub pending: usize,
    /// The game being played, as "3 of 40 unlocked".
    pub game: Option<String>,
    /// How many of the card's games are saved for offline play, as "12 of 40 saved".
    pub library: Option<String>,
}

pub fn tail_text(status: &NetStatus) -> String {
    match &status.tail {
        TailState::Off => "Off".into(),
        TailState::Missing => "Not on this card".into(),
        TailState::WaitingForWifi => "Waiting for Wi-Fi".into(),
        TailState::Starting => "Starting".into(),
        TailState::NeedsLogin { .. } => "Log in to connect".into(),
        TailState::NeedsApproval => "Waiting for approval".into(),
        TailState::Running { .. } => "Connected".into(),
        TailState::Failed(e) => e.clone(),
    }
}

pub fn ach_text(ach: &AchView) -> String {
    match ach.enabled {
        false => "Off".into(),
        true => ach.state.clone(),
    }
}

pub fn sync_text(status: &NetStatus) -> String {
    match &status.sync {
        SyncState::Off => "Off".into(),
        SyncState::Missing => "Not on this card".into(),
        SyncState::WaitingForWifi => "Waiting for Wi-Fi".into(),
        SyncState::Starting => "Starting".into(),
        SyncState::Paused => "Paused while playing".into(),
        SyncState::Running { busy: true, .. } => "Syncing".into(),
        SyncState::Running { devices: 0, .. } => "No devices".into(),
        SyncState::Running { devices: 1, .. } => "1 device".into(),
        SyncState::Running { devices, .. } => format!("{devices} devices"),
        SyncState::Failed(e) => e.clone(),
    }
}

/// A Syncthing device ID, two of its groups to a line so it fits beside the QR code.
pub fn id_lines(id: &str) -> Vec<String> {
    let groups: Vec<&str> = id.split('-').collect();
    groups.chunks(2).map(|g| g.join("-")).collect()
}

/// One line of an info screen; `gap` is the space above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoLine {
    pub face: PageFace,
    pub gap: u32,
}

impl InfoLine {
    pub fn label(t: &str) -> InfoLine {
        InfoLine {
            face: PageFace::Small(t.to_string()),
            gap: 14,
        }
    }
    pub fn value(t: &str) -> InfoLine {
        InfoLine {
            face: PageFace::Value(t.to_string(), true),
            gap: 2,
        }
    }
    pub fn mono(t: &str) -> InfoLine {
        InfoLine {
            face: PageFace::Mono(t.to_string()),
            gap: 2,
        }
    }
}

/// The pairing screen's layout for slot's own pages: a QR code on the left with the lines
/// beside it, or the lines centred when there is no code to show.
pub struct InfoScreen<'a> {
    pub qr: Option<String>,
    pub lines: &'a [InfoLine],
    pub ground: Option<(TexId, u32)>,
    pub canvas: Canvas,
    pub faces: &'a PageFaces,
}

/// The widest line beside the code: what an info screen's text is cut to.
pub fn info_width(canvas: Canvas) -> f32 {
    pair_text_width(canvas)
}

impl InfoScreen<'_> {
    pub fn draw(&self, out: &mut Vec<Draw>) {
        draw_menu_ground(self.canvas, self.ground, out);
        let qr = self
            .qr
            .as_ref()
            .and_then(|d| self.faces.get(PageFace::Qr(d.clone())));
        let faces: Vec<_> = self
            .lines
            .iter()
            .map(|l| (l.gap as f32, self.faces.get(l.face.clone())))
            .collect();
        let height: f32 = faces
            .iter()
            .map(|(g, f)| g + f.map_or(0.0, |f| f.2 as f32))
            .sum();
        if let Some(face) = qr {
            let inset = (PAIR_QR.saturating_sub(face.1) / 2) as f32;
            push(out, face, PAIR_X + inset, PAIR_TOP + inset);
        }
        let with_code = self.qr.is_some();
        let max_w = match with_code {
            true => info_width(self.canvas),
            false => self.canvas.w - 2.0 * PAIR_X,
        };
        let mut y = match with_code {
            true => PAIR_TOP + ((PAIR_QR as f32 - height) / 2.0).max(0.0),
            false => ((self.canvas.h - crate::slot_chrome::MOUTH_H - height) / 2.0).max(PAIR_TOP),
        };
        for (gap, face) in faces {
            y += gap;
            let Some(face) = face else {
                continue;
            };
            let w = (face.1 as f32).min(max_w);
            let x = match with_code {
                true => PAIR_X + PAIR_QR as f32 + PAIR_GAP,
                false => self.canvas.centred(w),
            };
            let s = w / face.1 as f32;
            out.push(Draw::Tex {
                x: x.round(),
                y: y.round(),
                w,
                h: face.2 as f32 * s,
                tex: face.0,
                alpha: 1.0,
            });
            y += face.2 as f32 * s;
        }
    }
}

fn push(out: &mut Vec<Draw>, (tex, w, h): (TexId, u32, u32), x: f32, y: f32) {
    out.push(Draw::Tex {
        x: x.round(),
        y: y.round(),
        w: w as f32,
        h: h as f32,
        tex,
        alpha: 1.0,
    });
}
