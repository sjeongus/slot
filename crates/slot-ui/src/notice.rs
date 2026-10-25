//! The banner at the top of the screen for achievements: what happened, its name, a detail,
//! and its badge. It slides down from the top edge and back.

use std::path::Path;

use crate::draw::{Canvas, Draw};
use crate::net_screen::{NetFace, NetFaces};

const W_MAX: f32 = 540.0;
const H: f32 = 84.0;
const TOP: f32 = 10.0;
const PAD: f32 = 12.0;
pub const BADGE: u32 = 60;
const FILL: [f32; 4] = [0.07, 0.07, 0.09, 0.94];
const EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.16];

#[allow(clippy::too_many_arguments)]
pub fn draw_notice(
    canvas: Canvas,
    heading: &str,
    name: &str,
    detail: Option<&str>,
    badge: Option<&Path>,
    shown: f32,
    faces: &NetFaces,
    out: &mut Vec<Draw>,
) {
    let shown = shown.clamp(0.0, 1.0);
    let eased = shown * shown * (3.0 - 2.0 * shown);
    let w = W_MAX.min(canvas.w - 32.0);
    let x = canvas.centred(w);
    let y = (TOP + H) * eased - H;
    // Ask for every face first, so one frame requests them all.
    let badge = badge.and_then(|p| faces.get(NetFace::Badge(p.to_path_buf())));
    let heading = faces.get(NetFace::Small(heading.to_string()));
    let name = faces.get(NetFace::Value(name.to_string(), true));
    let detail = detail.and_then(|d| faces.get(NetFace::Small(d.to_string())));
    out.push(Draw::Rect {
        x: x - 1.0,
        y: y - 1.0,
        w: w + 2.0,
        h: H + 2.0,
        colour: EDGE,
    });
    out.push(Draw::Rect {
        x,
        y,
        w,
        h: H,
        colour: FILL,
    });
    let mut tx = x + PAD;
    if let Some(b) = badge {
        push(out, b, tx, y + (H - b.2 as f32) / 2.0);
        tx += b.1 as f32 + PAD;
    }
    let lines: Vec<_> = [heading, name, detail].into_iter().flatten().collect();
    let total: f32 = lines.iter().map(|l| l.2 as f32).sum();
    let mut ly = y + (H - total) / 2.0;
    for l in lines {
        push(out, l, tx, ly);
        ly += l.2 as f32;
    }
}

fn push(out: &mut Vec<Draw>, t: (crate::draw::TexId, u32, u32), x: f32, y: f32) {
    out.push(Draw::Tex {
        x: x.round(),
        y: y.round(),
        w: t.1 as f32,
        h: t.2 as f32,
        tex: t.0,
        alpha: 1.0,
    });
}

/// A badge image from the card, scaled to the notice's square.
pub fn achievement_badge_face(path: &Path) -> crate::plate::UndoFace {
    let empty = crate::plate::UndoFace {
        rgba: Vec::new(),
        w: 0,
        h: 0,
    };
    let Ok(file) = std::fs::File::open(path) else {
        return empty;
    };
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let Ok(mut reader) = dec.read_info() else {
        return empty;
    };
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return empty;
    };
    let (sw, sh) = (info.width as usize, info.height as usize);
    if sw == 0 || sh == 0 {
        return empty;
    }
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 1,
    };
    let px = |x: usize, y: usize| -> [u8; 4] {
        let at = (y * sw + x) * channels;
        let p = &buf[at..at + channels];
        match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        }
    };
    let n = BADGE as usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let p = px(x * sw / n, y * sh / n);
            rgba[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&p);
        }
    }
    crate::plate::UndoFace {
        rgba,
        w: BADGE,
        h: BADGE,
    }
}
