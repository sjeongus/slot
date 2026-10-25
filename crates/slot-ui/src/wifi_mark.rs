//! The Wi-Fi mark beside the clock while slot is online: a dot and three arcs, drawn smooth at
//! the screen's own resolution. The symbols font on the card has no Wi-Fi glyph, so the shape
//! is computed here. One texture per ink, made once by the frontend when a draw first asks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::draw::{Draw, TexId};
use crate::plate::UndoFace;

static ONLINE: AtomicBool = AtomicBool::new(false);

/// slot sets this each frame from the network thread's status.
pub fn set_online(online: bool) {
    ONLINE.store(online, Ordering::Relaxed);
}

pub fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

/// Inks the mark has been drawn in, and their textures once the frontend has made them.
type Faces = Vec<([u8; 3], Option<(TexId, u32, u32)>)>;
static FACES: Mutex<Faces> = Mutex::new(Vec::new());

/// Space between the mark and the clock.
pub const MARK_GAP: f32 = 8.0;

const W: u32 = 30;
const H: u32 = 22;
/// The arcs fan out from this point, a little above the bottom edge.
const APEX: (f32, f32) = (W as f32 / 2.0, H as f32 - 2.6);
const DOT_R: f32 = 2.4;
/// Inner and outer radius of each arc.
const ARCS: [(f32, f32); 3] = [(5.6, 8.4), (11.0, 13.8), (16.4, 19.2)];
/// Arcs span 45° either side of straight up.
const SPREAD: f32 = std::f32::consts::FRAC_PI_4;

fn inside(x: f32, y: f32) -> bool {
    let (dx, dy) = (x - APEX.0, APEX.1 - y);
    let r = (dx * dx + dy * dy).sqrt();
    if r <= DOT_R {
        return true;
    }
    if dy <= 0.0 || dx.abs().atan2(dy) > SPREAD {
        return false;
    }
    ARCS.iter().any(|(a, b)| r >= *a && r <= *b)
}

/// The mark in `ink`, anti-aliased by sampling each pixel 4 × 4 times.
pub fn wifi_mark_face(ink: [u8; 3]) -> UndoFace {
    const N: u32 = 4;
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for py in 0..H {
        for px in 0..W {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                    hits += inside(x, y) as u32;
                }
            }
            let at = ((py * W + px) * 4) as usize;
            let a = (hits * 255 / (N * N)) as u8;
            rgba[at..at + 4].copy_from_slice(&[ink[0], ink[1], ink[2], a]);
        }
    }
    UndoFace { rgba, w: W, h: H }
}

/// Inks asked for that have no texture yet. The frontend makes them with
/// [`wifi_mark_face`] and hands them back through [`set_wifi_mark_face`].
pub fn wifi_mark_wanted() -> Vec<[u8; 3]> {
    let faces = FACES.lock().unwrap_or_else(|p| p.into_inner());
    faces
        .iter()
        .filter(|(_, t)| t.is_none())
        .map(|(ink, _)| *ink)
        .collect()
}

pub fn set_wifi_mark_face(ink: [u8; 3], tex: TexId, w: u32, h: u32) {
    let mut faces = FACES.lock().unwrap_or_else(|p| p.into_inner());
    match faces.iter_mut().find(|(i, _)| *i == ink) {
        Some(entry) => entry.1 = Some((tex, w, h)),
        None => faces.push((ink, Some((tex, w, h)))),
    }
}

/// Forgets every texture, for a renderer that has lost or recreated its textures.
pub fn reset_wifi_mark_faces() {
    FACES.lock().unwrap_or_else(|p| p.into_inner()).clear();
}

/// Draws the mark with its right edge at `right`, centred on `mid`, when slot is online.
pub fn draw_wifi_mark(right: f32, mid: f32, ink: [u8; 3], out: &mut Vec<Draw>) {
    if !online() {
        return;
    }
    let mut faces = FACES.lock().unwrap_or_else(|p| p.into_inner());
    let face = match faces.iter().find(|(i, _)| *i == ink) {
        Some((_, face)) => *face,
        None => {
            faces.push((ink, None));
            None
        }
    };
    let Some((tex, w, h)) = face else {
        return;
    };
    out.push(Draw::Tex {
        x: (right - w as f32).round(),
        y: (mid - h as f32 / 2.0).round(),
        w: w as f32,
        h: h as f32,
        tex,
        alpha: 1.0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_smooth_and_symmetric() {
        let f = wifi_mark_face([255, 255, 255]);
        let alpha = |x: u32, y: u32| f.rgba[((y * f.w + x) * 4 + 3) as usize];
        let partial = (0..f.h)
            .flat_map(|y| (0..f.w).map(move |x| (x, y)))
            .filter(|&(x, y)| (1..255).contains(&alpha(x, y)))
            .count();
        assert!(partial > 40, "edges are anti-aliased: {partial}");
        for y in 0..f.h {
            for x in 0..f.w {
                assert_eq!(alpha(x, y), alpha(f.w - 1 - x, y), "mirrored at {x},{y}");
            }
        }
        assert_eq!(alpha(f.w / 2, APEX.1 as u32), 255, "the dot is solid");
        assert_eq!(alpha(0, f.h - 1), 0, "the corners are clear");
    }
}
