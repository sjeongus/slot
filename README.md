<div align="center">

<img src=".github/resources/banner.png" width="100%" alt="slot. A bespoke, Game Boy-centric frontend for the Anbernic RG SP.">

<a href="LICENSE"><img src="https://img.shields.io/github/license/BrandonKowalski/slot?style=for-the-badge&labelColor=242429&color=4d4d57" height="36" alt="License"></a>
<a href="https://github.com/BrandonKowalski/slot/stargazers"><img src="https://img.shields.io/github/stars/BrandonKowalski/slot?style=for-the-badge&labelColor=242429&color=4d4d57" height="36" alt="Stars"></a>
<a href="https://github.com/BrandonKowalski/slot/releases"><img src="https://img.shields.io/github/downloads/BrandonKowalski/slot/total?style=for-the-badge&labelColor=242429&color=4d4d57" height="36" alt="Downloads"></a>
<a href="https://slot-cfw.fyi"><img src="https://img.shields.io/badge/Guide-slot--cfw.fyi-4d4d57?style=for-the-badge&labelColor=242429" height="36" alt="Guide"></a>
<a href="CHANGELOG.md"><img src="https://img.shields.io/badge/Changelog-4d4d57?style=for-the-badge&labelColor=242429" height="36" alt="Changelog"></a>

</div>

---

# About this fork

This is [slot](https://github.com/BrandonKowalski/slot) with **Wi-Fi and Syncthing** added.
Everything else is upstream slot, kept as close to it as possible so new releases merge in
cleanly.

**Why it exists:** I play the same games across several devices, and I want my saves to follow
me. Upstream slot has no way to join a Wi-Fi network; it only uses Wi-Fi for link play. So
there's no way to sync saves without pulling the SD card. This fork adds:

- **Settings > Network.** Scan for networks, choose one, and type its password on an on-screen
  keyboard driven by the D-pad or stick. Saved networks reconnect whenever the device is on, and
  hidden networks are supported.
- **[Syncthing](https://syncthing.net)**, bundled. One switch keeps `Saves/` and `States/` in
  step with your other devices. The device ID shows as a QR code to scan from the Syncthing app.
  Syncing pauses while a game is running, so another device's save can't land under it.
- **Link play still works.** Starting a link sets home Wi-Fi and Syncthing aside, and ending it
  brings them back.
- **RetroArch-compatible GBA saves and states.** Battery saves go to `Saves/mGBA/<game>.srm`, and
  states to `States/mGBA/<game>.state`, `.state1`, … and `.state.auto`, so they load in RetroArch
  on other devices and the other way round. On insert, slot resumes from the newest state,
  whichever device wrote it.
- **Network time.** The clock sets itself over NTP. The time zone is found automatically, with
  daylight saving followed, so state timestamps are right.

Set-up, where things are stored, and how upstream releases are merged are in
**[FORK.md](FORK.md)**. Builds come from this repo's
[Actions](https://github.com/sjeongus/slot/actions/workflows/release.yml) as `slot-*-h700.zip`,
installed the same way as upstream slot.

Built for the RG35XX-H. Not affiliated with or supported by the upstream author, so please file
problems with the network features here rather than upstream.

---

# What is slot?

A bespoke, Game Boy-centric frontend for the Anbernic RG SP.

Has support for GBA, GBC, and GB titles only.

> [!IMPORTANT]
> ROMs must be unzipped. slot does not read `.zip` or `.7z` files.

A full user guide can be found at [slot-cfw.fyi](https://slot-cfw.fyi).

Release notes can be found in the [changelog](CHANGELOG.md).

---

# What does it look like?

<p align="center">
<img src=".github/resources/screenshots/carousel.gif" width="48%" alt="Scrolling the cart carousel across the Game Boy Advance, Game Boy and Game Boy Color shelves">
<img src=".github/resources/screenshots/core-picker.gif" width="48%" alt="Flipping a cart to its circuit board to choose mGBA or gpSP">
</p>

<p align="center">
<img src=".github/resources/screenshots/link.gif" width="96.5%" alt="Two SPs racing in Mario Kart over the emulated link">
</p>

---

# Supported Devices

| Device             | Supported                                    | Since Version |
|--------------------|----------------------------------------------|---------------|
| Anbernic RG SP     | Yes                                          | Always        |
| Anbernic RG34XX    | Yes                                          | 1.4.0         |
| Anbernic RG34XXSP  | Yes                                          | 1.4.0         |
| Anbernic RG35XXSP  | Yes                                          | 1.5.0         |
| Anbernic 35XX Plus | Yes                                          | 1.5.0         |
| Anbernic RG28XX    | Yes                                          | 1.6.0         |
| Anbernic RG35XX-H  | Yes                                          | 1.7.0         |

If you have one of the untested devices, or have tried slot on a device not listed,
please [open an issue on GitHub](https://github.com/BrandonKowalski/slot/issues) and let me know how it went.

---

# AI Disclosure

The Rust frontend, and the SVGs on [slot-cfw.fyi](https://slot-cfw.fyi), were put together by Claude.
I reviewed everything that was produced. All documentation is 100% free-range, meatbag prose.

My goal with this project was to recreate the feeling of playing my GBA SP in the back seat as a kid,
but with a few modern creature comforts.

Nothing about this project is exciting or groundbreaking from a technological perspective, and as a
result, I delegated the glue code. mGBA and gpSP have been around forever and are the true stars of
this show. The incredible skeuomorphic art is the work of [Markus](https://github.com/MarkusMaribu), an
artist who saw the bare-bones version and offered to dress it up.

I understand if my use of AI is a dealbreaker for you. Just know it's the least interesting part of
what's here.

Figured I should share the end result of all the wasted water. ✌🏻

---

# Spread Joy!

I've spent a lot of time building slot.

If you enjoy using it and feel inclined to pay it forward, go do something nice for someone! ❤️