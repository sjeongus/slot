use std::path::{Path, PathBuf};

use slot_store::Platform;

pub const BEZELS: &str = "System/bezels";
pub const GAME_BEZELS: &str = "Bezels";

pub fn list(root: &Path, platform: Platform) -> Vec<String> {
    let dir = root.join(GAME_BEZELS).join(platform.dir_name());
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !slot_store::is_hidden(p))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .filter_map(|p| Some(p.file_name()?.to_str()?.to_owned()))
        .collect();
    files.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    files
}

fn dims(path: &Path) -> Option<(u32, u32)> {
    if !path.extension()?.eq_ignore_ascii_case("png") {
        return None;
    }
    let (w, h) = path.file_stem()?.to_str()?.split_once('x')?;
    let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

pub fn pick(root: &Path, panel: (u32, u32)) -> Option<PathBuf> {
    let mut fits: Vec<((u32, u32), PathBuf)> = std::fs::read_dir(root.join(BEZELS))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| !slot_store::is_hidden(p))
        .filter_map(|p| Some((dims(&p)?, p)))
        .filter(|((w, h), _)| {
            u64::from(*w) * u64::from(panel.1) == u64::from(*h) * u64::from(panel.0)
        })
        .collect();
    fits.sort_by_key(|((w, _), p)| (w.abs_diff(panel.0), p.clone()));
    fits.into_iter().next().map(|(_, p)| p)
}

/// Whether a bezel drawn for this panel wants the game at the top of the screen, as the
/// AGS-102 frame has it, rather than centred. Read from the bezel itself: the clear window down
/// its middle column is compared with where the game sits either way.
pub fn wants_game_on_top(rgba: &[u8], w: u32, h: u32) -> bool {
    if w == 0 || rgba.len() < (w * h * 4) as usize {
        return false;
    }
    let x = (w / 2) as usize;
    let clear = |y: u32| rgba[(y as usize * w as usize + x) * 4 + 3] < 32;
    let (Some(first), Some(last)) = ((0..h).find(|&y| clear(y)), (0..h).rev().find(|&y| clear(y)))
    else {
        return false;
    };
    let window = f64::from(first + last) / 2.0;
    let middle = |(_, y, _, gh): (i32, i32, i32, i32)| f64::from(y) + f64::from(gh) / 2.0;
    let top = middle(slot_gfx::game_rect((w, h), true));
    let centred = middle(slot_gfx::game_rect((w, h), false));
    (window - top).abs() < (window - centred).abs()
}

#[cfg(test)]
mod window_tests {
    use super::wants_game_on_top;

    /// A 640 x 480 bezel, opaque but for a clear window `rows` down the middle.
    fn bezel(rows: std::ops::Range<u32>) -> Vec<u8> {
        let (w, h) = (640u32, 480u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in rows {
            for x in 0..w {
                rgba[((y * w + x) * 4 + 3) as usize] = 0;
            }
        }
        rgba
    }

    #[test]
    fn a_window_at_the_top_moves_the_game_up() {
        assert!(wants_game_on_top(&bezel(0..427), 640, 480));
        assert!(
            wants_game_on_top(&bezel(21..405), 640, 480),
            "a GB window, top layout"
        );
    }

    #[test]
    fn a_centred_window_keeps_the_game_centred() {
        assert!(!wants_game_on_top(&bezel(27..454), 640, 480));
        assert!(
            !wants_game_on_top(&bezel(48..432), 640, 480),
            "a GB window, centred"
        );
        assert!(
            !wants_game_on_top(&bezel(0..0), 640, 480),
            "no window at all"
        );
    }
}
