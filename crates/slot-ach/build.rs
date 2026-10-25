//! Builds the vendored rcheevos (MIT, see rcheevos/LICENSE) and slot's C glue into one static
//! library. Disc, zip and encrypted-ROM hashing are left out: slot only plays GB, GBC and GBA.

fn main() {
    let rc = std::path::Path::new("rcheevos");
    let src = rc.join("src");
    let mut files = vec![std::path::PathBuf::from("glue/glue.c")];
    for f in [
        "rc_client.c",
        "rc_compat.c",
        "rc_libretro.c",
        "rc_util.c",
        "rc_version.c",
        "rhash/hash.c",
        "rhash/hash_rom.c",
        "rhash/md5.c",
    ] {
        files.push(src.join(f));
    }
    for dir in ["rapi", "rcheevos"] {
        let mut more: Vec<_> = std::fs::read_dir(src.join(dir))
            .expect("rcheevos sources")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "c"))
            .collect();
        more.sort();
        files.extend(more);
    }
    let mut build = cc::Build::new();
    build
        .files(&files)
        .include(rc.join("include"))
        .include(&src)
        .include("glue")
        .define("RC_DISABLE_LUA", None)
        .define("RC_CLIENT_SUPPORTS_HASH", None)
        .define("RC_HASH_NO_DISC", None)
        .define("RC_HASH_NO_ZIP", None)
        .define("RC_HASH_NO_ENCRYPTED", None)
        .warnings(false)
        .opt_level(2);
    build.compile("rcheevos");
    println!("cargo:rerun-if-changed=glue");
    println!("cargo:rerun-if-changed=rcheevos");
}
