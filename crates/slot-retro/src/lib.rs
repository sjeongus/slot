macro_rules! eprintln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

mod core;
mod ffi;
mod libretro;
mod link;
mod mock;
mod rumble;

pub use core::{AvInfo, ButtonMask, CoreError, MemoryDescriptor, RetroCore, GBA_H, GBA_W};
pub use ffi::{
    MEMORY_SAVE_RAM, MEMORY_SYSTEM_RAM, NETPACKET_BROADCAST, NETPACKET_FLUSH_HINT,
    NETPACKET_RELIABLE, NETPACKET_UNRELIABLE, NETPACKET_UNSEQUENCED,
};
pub use libretro::LibretroCore;
pub use link::{Link, LinkChannel, LoopbackLink};
pub use mock::MockCore;
pub use rumble::Rumble;
