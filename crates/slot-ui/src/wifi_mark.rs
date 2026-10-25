//! The Wi-Fi mark beside the clock while slot is online. Drawn from rectangles, so it needs
//! no texture and takes the ink of whatever it sits beside.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::draw::Draw;

static ONLINE: AtomicBool = AtomicBool::new(false);

/// slot sets this each frame from the network thread's status.
pub fn set_online(online: bool) {
    ONLINE.store(online, Ordering::Relaxed);
}

pub fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

const MARK: [&str; 12] = [
    ".....XXXXX.....",
    "..XXXXXXXXXXX..",
    ".XXX.......XXX.",
    "XX...........XX",
    "....XXXXXXX....",
    "..XXXX...XXXX..",
    ".XX.........XX.",
    ".....XXXXX.....",
    "....XX...XX....",
    "...............",
    "......XXX......",
    "......XXX......",
];
const SCALE: f32 = 2.0;
/// Space between the mark and the clock.
pub const MARK_GAP: f32 = 8.0;

pub fn mark_width() -> f32 {
    MARK[0].len() as f32 * SCALE
}

/// Draws the mark with its right edge at `right`, centred on `mid`, when slot is online.
pub fn draw_wifi_mark(right: f32, mid: f32, ink: [u8; 3], out: &mut Vec<Draw>) {
    if !online() {
        return;
    }
    let colour = [
        f32::from(ink[0]) / 255.0,
        f32::from(ink[1]) / 255.0,
        f32::from(ink[2]) / 255.0,
        1.0,
    ];
    let left = (right - mark_width()).round();
    let top = (mid - MARK.len() as f32 * SCALE / 2.0).round();
    for (y, row) in MARK.iter().enumerate() {
        let bytes = row.as_bytes();
        let mut x = 0;
        while x < bytes.len() {
            if bytes[x] != b'X' {
                x += 1;
                continue;
            }
            let start = x;
            while x < bytes.len() && bytes[x] == b'X' {
                x += 1;
            }
            out.push(Draw::Rect {
                x: left + start as f32 * SCALE,
                y: top + y as f32 * SCALE,
                w: (x - start) as f32 * SCALE,
                h: SCALE,
                colour,
            });
        }
    }
}
