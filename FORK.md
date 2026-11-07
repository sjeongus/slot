# slot, with Syncthing, Tailscale, RetroAchievements and RetroArch-compatible saves

This is a fork of [slot-cfw/slot](https://github.com/slot-cfw/slot). It keeps `Saves/` and
`States/` in step with your other devices through [Syncthing](https://syncthing.net), puts the
device on your [Tailscale](https://tailscale.com) tailnet, and earns
[RetroAchievements](https://retroachievements.org), online or off. GBA saves and states are kept
in RetroArch's layout, so they work with RetroArch on those devices. Over-the-air updates come
from this fork's releases. Everything else, Wi-Fi and the clock included, is upstream slot, and
new upstream releases are merged in as they come.

## Using it

Wi-Fi is upstream's: **General > Wi-Fi** turns it on and picks a network. This fork adds three
rows to that page, **Syncthing**, **Tailscale** and **Achievements**. Each opens a page of its
own, drawn like the rest of the menu. Achievements stays on the page with Wi-Fi off, for
offline play.

1. Open the menu on the shelf (MENU), then **General > Wi-Fi**, and join your network.
2. **Syncthing**: turn **Syncthing** on. Once the device is on Wi-Fi, **Device ID** shows:
   - this device's ID, as text and as a QR code you can scan from the Syncthing app on a phone;
   - the address, user and password of the Syncthing web interface.
3. On your computer, add the handheld as a device in Syncthing using that ID. Then accept the two
   folders it offers:

   | Folder ID     | On the card | Share it with                                      |
   |---------------|-------------|----------------------------------------------------|
   | `slot-saves`  | `Saves/`    | RetroArch's saves folder (5 old versions kept on the card) |
   | `slot-states` | `States/`   | RetroArch's states folder                          |

   This only works if RetroArch on your other devices sorts saves and states into folders by
   core. That's RetroArch's default (`sort_savefiles_enable`, `sort_savestates_enable`). The
   game's ROM must also have the same file name on every device.

4. **Sync In Games**, also on the Syncthing page, is off by default. While a cart is in, Syncthing pauses both folders, so a
   save from another device can't land under a running game, and the emulator keeps the CPU.
   Ejecting resumes them and rescans. With it on, slot keeps Wi-Fi up while a game runs, which
   upstream otherwise turns off to save power.

### Saves and states (RetroArch format)

With **Settings > Gameplay > Save Format** set to **RetroArch** (the default), GBA games use
RetroArch's mGBA layout. The row is only on the shelf's menu, not the in-game one, so the layout
can't change under a running game.

| What | On the card |
|---|---|
| Battery save | `Saves/mGBA/<game>.srm` |
| Save states | `States/mGBA/<game>.state`, `.state1`, `.state2`, … with a `.png` thumbnail beside each |
| Resume state | `States/mGBA/<game>.state.auto` |

The states are the mGBA core's own data, the same thing RetroArch stores, so they load in either
direction. slot reads RetroArch's compressed (rzip, deflate or zstd) and `RASTATE` files, and
writes plain ones, which every RetroArch version loads.

- **Loading on insert:** with **Load State on Insert** on, slot loads the **newest state by time**.
  That's `.state.auto` or any numbered slot, from whichever device wrote it last. SELECT+L1
  does the same.
- **The state switcher** lists `.state.auto` first, then numbered slots from the highest down
  (`.state12`, `.state11`, …, `.state`). There is no limit of ten.
- **Saving a state** writes one slot above the highest, as RetroArch's auto-increment does, so
  nothing is overwritten.
- **A state the core refuses** is moved to `States/.slot/`, which isn't synced, and the next start
  uses the next newest.
- **Older saves:** slot's own files from before (`Saves/GBA/*.sav`, `States/GBA/…`) are left
  where they are. A game with no `.srm` yet still loads its old `.sav`, and its first save writes
  the `.srm`.
- **gpSP:** if you switch a game to gpSP, its states go to `States/gpSP/`, as RetroArch's would.
  Its battery save stays in `mGBA/`, so changing cores never hides it.
- **GB and GBC:** these keep slot's own layout.
- **Save Format → slot** goes back to slot's own layout for everything.

### Tailscale

1. General > Wi-Fi > **Tailscale**: turn **Tailscale** on. It starts once Wi-Fi is connected.
2. **Log In** (shown while Tailscale waits for one) opens a code and a link. Scan the code with your phone, or type the link into a
   browser anywhere, and log in to Tailscale. The page moves on by itself once it has worked.
3. **Address** and **Device Name** then show how to reach the handheld on your tailnet. The
   name is `slot-` and the model, such as `slot-rg35xxh`.

Notes:

- The device keeps its identity in `Config/tailscale/`, so it stays the same device when you
  reflash BaseOS. Delete that folder, or use **Log Out**, to remove it.
- BaseOS's kernel has TUN built in, so Tailscale makes a real `tailscale0` interface and
  everything on the handheld can use the tailnet, Syncthing included: add the handheld to
  Syncthing elsewhere by its Tailscale address to sync away from home.
- slot leaves DNS and the firewall to BaseOS (`--accept-dns=false`, `--netfilter-mode=off`).
- Like Syncthing, Tailscale stops for link play and starts again afterwards.
- While Tailscale is on, slot keeps Wi-Fi up during games, so the device stays reachable.
- Tailscale's daemon logs to `Logs/tailscale.log`.

### RetroAchievements

1. General > Wi-Fi > **Achievements** > **Account**. Type your RetroAchievements user name, then your
   password, on upstream's keyboard (START goes on to the next step). Only the login token that comes back is kept, in `Config/achievements.txt`; the
   password is not.
2. Start a game. A notice shows how many of its achievements you have, and each unlock pops up
   over the game with its badge.

Notes:

- **Softcore only.** RetroAchievements grants hardcore only to emulators it has approved, and
  this fork is not one. Save states, rewind and fast-forward all stay available.
- **Offline**, in the manner of [RAOfflineProxy](https://github.com/misantronic/RAOfflineProxy):
  - The first time a game is played online, slot keeps its achievement set and your unlocks on
    the card (`Config/achievements/`).
  - Offline, that game's achievements still load and unlock. Each unlock is saved on the card
    ("Saved, and sent once back online"), so a power-off does not lose it.
  - Back online, saved unlocks are sent oldest first, each with how long ago it was earned
    (RetroAchievements accepts up to 14 days). **Saved Unlocks** shows how many are waiting;
    A sends them now.
  - **Offline Games** saves every game on the card ahead of time. While slot is online, logged
    in and not running a game, it works through the card's ROMs in the background: each one's
    achievement set and your unlocks in it, at most 100 new games per half hour. The row shows
    how many are saved. Each ROM is hashed once (`Config/achievements/hashes.txt`) and again
    only if the file changes.
  - A game that was not saved yet says "Not saved for offline play yet" offline; one that
    RetroAchievements does not know says "None for this game".
- Loading a state or rewinding restarts achievement tracking from that point, as RetroArch does
  in softcore.
- The page opens offline too, so achievements can be turned on or off anywhere.
- While logged in, slot keeps Wi-Fi up during games, so unlocks are sent as they happen; with
  no network they are saved instead.
- Requests go through BaseOS's own `curl`, with its CA certificates.
- Achievement badges are saved in `Config/achievements/badges/` when a game loads online, so
  they also show offline.

### The keyboard

Upstream's keyboard, with one button added: **R2** flips to the symbols page and back, as the
`#+=` key does. The rest is upstream's: B deletes, X types a space, SELECT is Shift, L1 and R1
move the cursor, Y cancels, START finishes, MENU shows the help.

### Updates

**Tools > Check for Updates** reads `versions.json` from this fork's `net` branch
(`raw.githubusercontent.com/sjeongus/slot/net/versions.json`) and offers this fork's releases,
never upstream's, which would drop these features. The zip comes from this fork's GitHub release
and is checked against the sha256 in the manifest. Versions compare as `1.14.0` <
`1.14.0-net.1` < `1.14.0-net.2` < `1.15.0-net.1`.

The release workflow writes `versions.json` once a release's zip is up and commits it to `net`,
so nothing needs doing by hand.

BaseOS is never updated over the air in this fork: the check leaves it out, and the manifest
has no BaseOS entry. This fork's BaseOS (with the charging screen) goes on from the card: copy
its `baseos-rg35xxh-<version>.bosupd` to the root of the SD card and restart. BaseOS applies it
at boot, as it does any update file it finds there.

### Link play

Linking needs the radio to itself. When you start a link, Syncthing and Tailscale stop with
upstream's Wi-Fi, and start again once it is back. Nothing to do by hand.

On models whose BaseOS build has no Wi-Fi (the RG28XX), the Syncthing and Tailscale rows are
hidden.

### Settings file

Syncthing, Tailscale and Save Format are stored in `Config/net.txt`, which you can also edit on
a computer while slot is off:

```
syncthing=on
sync_in_game=off
tailscale=on
save_format=retroarch
```

Wi-Fi networks are upstream's, in `Config/wifi.conf`. Earlier releases of this fork kept them in
`net.txt` with network time and the time zone; those lines are now ignored, and General > Wi-Fi
and Date & Time take over.

RetroAchievements has a file of its own, `Config/achievements.txt` (`enabled`, `user`, `token`).

### Where Syncthing keeps things

- **Binary**: `System/bin/syncthing`, the pinned linux-arm64 release (see `SYNCTHING_VERSION` in
  `taskfile.yml`). If the card won't run it in place, slot copies it to `/tmp/slotnet` and runs it
  from there.
- **Keys and config**: `Config/syncthing/`. The device keeps its ID when you move the card or
  reflash BaseOS.
- **Database**: `/data/slot/syncthing` on BaseOS's ext4 partition (only an index; rebuilt if lost).
- **Log**: `Logs/syncthing.log`.

Syncthing runs at a lower CPU priority than slot. The web interface listens on port 8384 behind a
password that slot picks on first start.

## What changed, file by file

Kept small on purpose, so upstream merges stay easy:

| Where | What |
|---|---|
| `crates/slot-net/` | **New.** The thread that runs Syncthing and Tailscale and follows slot's Wi-Fi (`set_link`). |
| `crates/slot-net/src/tailscale.rs` | **New.** Runs `tailscaled`, reads `tailscale status`, logs in and out. |
| `crates/slot-ach/` | **New.** RetroAchievements: rcheevos v12.5.0 (MIT, vendored), C glue, the offline layer, badges. |
| `crates/slot-store/src/retroarch.rs` | **New.** RetroArch's save and state layout. |
| `crates/slot-ui/src/net_pages.rs`, `notice.rs` | **New.** Status text, the info screen with a QR code, achievement notices. |
| `crates/slot-ui/src/quick_menu.rs` | The new rows and their three pages; a `texts` field for their status. |
| `crates/slot-ui/src/keyboard.rs`, `bitmaps.rs` | R2 for symbols, with upstream's R2 legend. |
| `crates/slot-retro/src/libretro.rs`, `core.rs` | Keeps the core's memory map (`SET_MEMORY_MAPS`) for rcheevos. |
| `crates/slot/src/emu.rs` | ~20 lines: an achievements session per game, run after each frame, reset after a state load or rewind. |
| `crates/slot/src/app/net.rs` | **New.** The new rows and pages inside the App. |
| `crates/slot/src/app.rs` | Routes the new rows (`net_rows!`) and the NetInfo phase there; keeps Wi-Fi up in games while a service needs it. |
| `crates/slot/src/update.rs`, `.github/workflows/release.yml` | Updates from this fork; the workflow commits `versions.json`. |
| `crates/slot-gfx/src/headless_egl.rs` | **New.** A windowless GL surface, so the render tests also run on Linux. |
| `crates/slot/tests/network.rs`, `achievements_page.rs`, `render_fork.rs` | **New.** Tests; `render_fork` dumps the new screens with `SCRATCH_PNG_DIR`. |


## Taking new upstream releases

```sh
git fetch upstream --tags
git switch main && git merge --ff-only upstream/main   # main mirrors upstream, untouched
git switch net && git merge main                       # or: git merge v1.15.0
task test && task test:card
```

Conflicts, when they happen, are almost always in the lines listed above:

- **`QuickRow`**: keep this fork's rows (`SaveFormat` onwards) as the last variants of the enum
  and of `ALL`, and bump `ALL`'s length. The same for `QuickValue::RetroArch` and `SlotFormat`.
  New upstream rows go before them.
- **`app.rs`**: keep the `net_rows!()` arms at the top of `quick_value`, `open_quick_row` and
  `change_setting`, the `| QuickRow::SaveFormat` arms, `hidden.extend(self.net_hidden())`,
  and `net_wants_radio()` in `wifi_wanted`.
- **`emu.rs`**: keep `cheevos.frame()` right after the core runs a frame, `cheevos_stale` set
  wherever a state is loaded, and `drop(cheevos)` before the worker returns.

## Building

The same as upstream (`task dist:device`), plus `syncthing` and `tailscale` tasks that fetch the
pinned releases and check their checksums. `slot-ach` compiles rcheevos with the system C
compiler, which the device build's container already has. On the host build (`cargo run`), Syncthing and Tailscale are
simulated and follow upstream's simulated Wi-Fi.

To run slot-net's tests against a real Syncthing, set `SLOT_TEST_SYNCTHING` to a Syncthing binary
for your computer:

```sh
SLOT_TEST_SYNCTHING=/path/to/syncthing cargo test -p slot-net
```
