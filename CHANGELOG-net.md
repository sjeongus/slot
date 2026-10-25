# Changelog of the net fork

Releases of this fork are named after the upstream slot release they are built on, plus
`-net.N`. Everything in upstream's [CHANGELOG.md](CHANGELOG.md) for that version is included.

## [1.9.0-net.1](https://github.com/sjeongus/slot/releases/tag/v1.9.0-net.1)

Based on slot [1.9.0](https://github.com/BrandonKowalski/slot/releases/tag/v1.9.0). See
[FORK.md](https://github.com/sjeongus/slot/blob/net/FORK.md) for set-up.

**Network** (Settings > Network)
- Wi-Fi: scan, choose a network and type its password with the D-pad. Saved networks join by themselves whenever the device is on; hidden networks are supported.
- A Wi-Fi mark beside the clock while the device is online.
- Link play still works: it borrows the radio and gives it back afterwards.

**Syncthing**, bundled (v2.1.7), on a page of its own
- Keeps `Saves/` and `States/` in step with your other devices. The device ID shows as a QR code.
- Pauses while a game runs, unless Sync In Games is on.

**Tailscale**, bundled (1.102.3), on a page of its own
- Log in by scanning a code on screen or opening its link on another device. The address and device name show once connected.

**RetroAchievements**, softcore, on a page of its own
- Log in with the on-screen keyboard; only the login token is kept.
- Unlocks and game progress show over the game, with badges.
- Offline play: unlocks are saved on the card and sent once back online, each dated to when it was earned (up to 14 days).
- While online, every game on the card has its achievements saved ahead of time, so a game first started offline still has them.

**Saves**
- GBA saves and states use RetroArch's mGBA layout (`Saves/mGBA/<game>.srm`, `States/mGBA/<game>.state…`), so they sync with RetroArch. Settings > Gameplay > Save Format switches back to slot's own layout.
- The newest state loads on insert; the state switcher lists `.state.auto`, then the highest slot down, without a limit of ten.

**Clock**
- Network time over NTP, with the time zone and daylight saving found automatically.

**Fixes**
- The Gameplay menu scrolls once it has more rows than fit.
- On 640 x 480 screens, a custom bezel whose clear window is at the top moves the game up to it, as the AGS-102 frame does.
