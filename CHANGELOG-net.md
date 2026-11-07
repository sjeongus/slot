# Changelog of the net fork

Releases of this fork are named after the upstream slot release they are built on, plus
`-net.N`. Everything in upstream's [CHANGELOG.md](CHANGELOG.md) for that version is included.

## [1.14.0-net.1](https://github.com/sjeongus/slot/releases/tag/v1.14.0-net.1)

Based on slot [1.14.0](https://github.com/slot-cfw/slot/releases/tag/v1.14.0) and the changes
since, which bring upstream's own Wi-Fi, Gamebit, RomM, over-the-air updates, collections, time
zones with daylight saving, Gambatte for GB and GBC, and more: see [CHANGELOG.md](CHANGELOG.md).

**Now upstream's**
- Wi-Fi: General > Wi-Fi joins networks with upstream's keyboard, signal and lock icons. This fork's own Wi-Fi, network time, time zone and Wi-Fi mark are gone; upstream's Wi-Fi icon and Date & Time (Set Automatically) do the same jobs. Saved networks from earlier releases need joining once more.
- Bezels: upstream's rule decides when the game moves up under a custom bezel, and handles semi-transparent screens.

**Moved**
- Achievements, Syncthing and Tailscale are rows on General > Wi-Fi, each with its own page; Achievements stays there with Wi-Fi off. The pages look like the rest of the menu, and the device ID and the Tailscale login show on a screen like RomM's pairing one.
- Achievements can be opened, turned on and off with no network.

**New**
- Over-the-air updates come from this fork's releases (Tools > Check for Updates), never from upstream's, which would remove these features.
- R2 flips upstream's keyboard to the symbols page and back.
- Gameplay scrolls past six rows with a scroll bar, as General does.
- BaseOS is no longer updated over the air: this fork's BaseOS goes on from the SD card.
- With Achievements logged in, Tailscale on, or Syncthing's Sync In Games on, Wi-Fi stays up while a game runs; upstream turns it off then.

## [1.9.0-net.2](https://github.com/sjeongus/slot/releases/tag/v1.9.0-net.2)

- Custom bezels with a semi-transparent screen, such as an LCD grid over the game, now move the game up to the screen when it is at the top, as clear ones already did. Only solid rows count as frame.

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
