use std::path::{Path, PathBuf};

use slot_store::{atomic_write, write_slot_state, Core, Platform, SlotState, StateRing};

pub trait Snapshot {
    fn state(&self) -> Option<Vec<u8>>;
    fn save_ram(&self) -> Option<Vec<u8>>;
    fn thumb(&self) -> Option<Vec<u8>>;
    fn load(&self, state: Vec<u8>);

    fn resume_trusted(&self) -> bool {
        true
    }

    fn save_ram_trusted(&self) -> bool {
        true
    }
}

pub fn flush(
    root: &Path,
    platform: Platform,
    core: Core,
    stem: &str,
    state: Option<&[u8]>,
    sav: Option<&[u8]>,
) -> std::io::Result<()> {
    if let Some(state) = state {
        let ring = StateRing::new(root, platform, core, stem);
        if !matches!(ring.read_resume(), Ok(Some(old)) if old == state) {
            ring.write_resume(state)?;
        }
    }
    if let Some(sav) = sav {
        write_sav(root, platform, stem, sav)?;
    }
    Ok(())
}

pub fn eject(
    root: &Path,
    platform: Platform,
    core: Core,
    stem: &str,
    state: Option<&[u8]>,
    sav: Option<&[u8]>,
    slot: &SlotState,
) -> std::io::Result<()> {
    flush(root, platform, core, stem, state, sav)?;
    let mut slot = slot.clone();
    slot.cart = None;
    slot.cart_platform = None;
    write_slot_state(root, &slot)
}

pub fn write_sav(root: &Path, platform: Platform, stem: &str, sav: &[u8]) -> std::io::Result<bool> {
    let path = sav_path(root, platform, stem);
    let same = match slot_store::retroarch_save_path(root, platform, stem) {
        // RetroArch layout: only the file at the new path counts, so the first write after
        // switching lands there even when slot's old .sav holds the same bytes.
        Some(ra) => slot_store::read_decoded(&ra).is_ok_and(|old| old == sav),
        None => matches!(read_sav(root, platform, stem), Ok(Some(old)) if old == sav),
    };
    if same {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(&path, sav)?;
    Ok(true)
}

pub fn read_sav(root: &Path, platform: Platform, stem: &str) -> std::io::Result<Option<Vec<u8>>> {
    let srm = crate::root::saves_dir(root)
        .join(platform.dir_name())
        .join(format!("{stem}.srm"));
    if let Some(ra) = slot_store::retroarch_save_path(root, platform, stem) {
        // RetroArch's .srm first (it may be rzip compressed), then slot's older files.
        match slot_store::read_decoded(&ra) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    let legacy = crate::root::saves_dir(root)
        .join(platform.dir_name())
        .join(format!("{stem}.sav"));
    for path in [legacy, srm] {
        match std::fs::read(&path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

pub fn read_resume(
    root: &Path,
    platform: Platform,
    core: Core,
    stem: &str,
) -> std::io::Result<Option<Vec<u8>>> {
    StateRing::new(root, platform, core, stem).read_resume()
}

fn sav_path(root: &Path, platform: Platform, stem: &str) -> PathBuf {
    if let Some(ra) = slot_store::retroarch_save_path(root, platform, stem) {
        return ra;
    }
    crate::root::saves_dir(root)
        .join(platform.dir_name())
        .join(format!("{stem}.sav"))
}
