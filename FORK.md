# slot, with Wi-Fi, Syncthing, Tailscale, RetroAchievements and RetroArch-compatible saves

This is a fork of [BrandonKowalski/slot](https://github.com/BrandonKowalski/slot). It adds a
**Settings > Network** page. Use it to join Wi-Fi, typing the password with the D-pad, and to keep
`Saves/` and `States/` in step with your other devices through
[Syncthing](https://syncthing.net). It puts the device on your [Tailscale](https://tailscale.com)
tailnet and earns [RetroAchievements](https://retroachievements.org), online or off. GBA saves
and states are kept in RetroArch's layout, so they work with RetroArch on those devices. The
clock sets itself from the network, time zone and daylight saving included. A Wi-Fi mark beside
the clock shows when the device is online. Everything else is upstream slot, and new upstream
releases are merged in as they come.

## Using it

1. Open the menu on the shelf (MENU) and pick **Network**.
2. **Network**: press A to scan, choose your network, and type its password.
   - A types the highlighted key; B deletes; Y switches capitals; SELECT shows symbols; START
     joins.
   - Saved networks are joined automatically every time the device is on.
   - **Other Network…** at the bottom of the list joins a hidden network.
   The Network page also has **Syncthing**, **Tailscale** and **Achievements**, each opening a
   page of its own, and **Network Time** and **Time Zone**.
3. **Syncthing** opens its own page. Turn **Syncthing** on there. Once the device is on Wi-Fi,
   **Details** shows:
   - this device's ID, as text and as a QR code you can scan from the Syncthing app on a phone;
   - the address, user and password of the Syncthing web interface.
4. On your computer, add the handheld as a device in Syncthing using that ID. Then accept the two
   folders it offers:

   | Folder ID     | On the card | Share it with                                      |
   |---------------|-------------|----------------------------------------------------|
   | `slot-saves`  | `Saves/`    | RetroArch's saves folder (5 old versions kept on the card) |
   | `slot-states` | `States/`   | RetroArch's states folder                          |

   This only works if RetroArch on your other devices sorts saves and states into folders by
   core. That's RetroArch's default (`sort_savefiles_enable`, `sort_savestates_enable`). The
   game's ROM must also have the same file name on every device.

5. **Sync In Games**, also on the Syncthing page, is off by default. While a cart is in, Syncthing pauses both folders, so a
   save from another device can't land under a running game, and the emulator keeps the CPU.
   Ejecting resumes them and rescans.

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

1. Network → **Tailscale** → turn **Tailscale** on. It starts once Wi-Fi is connected.
2. **Log In** opens a code and a link. Scan the code with your phone, or type the link into a
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
- Tailscale's daemon logs to `Logs/tailscale.log`.

### RetroAchievements

1. Network → **Achievements** → **Account**. Type your RetroAchievements user name, then your
   password. Only the login token that comes back is kept, in `Config/achievements.txt`; the
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
- Requests go through BaseOS's own `curl`, with its CA certificates.
- Achievement badges are saved in `Config/achievements/badges/` when a game loads online, so
  they also show offline.

### Network time

- **Network Time** turns on BaseOS's time service. It sets the clock from `pool.ntp.org` once
  Wi-Fi is up and keeps it right; the row reads "On, synced" once it has.
- **Time Zone → Auto** finds your zone (e.g. `America/Los_Angeles`) from your connection the
  first time you're online. It asks ip-api.com, falling back to worldtimeapi.org; both see your
  public IP address.
  - The zone is remembered and checked again daily.
  - slot follows the zone's rules from BaseOS's time zone database, so daylight saving switches
    over by itself, even offline.
- **Time Zone → Manual** uses the offset you set under General → Date & Time, as upstream does.

### Link play

Linking needs the radio to itself. When you start a link, slot turns its own Wi-Fi and Syncthing
off. When the link ends, it turns them back on and rejoins your network. Nothing to do by hand.

On models whose BaseOS build has no Wi-Fi (the RG28XX), the Network row is hidden.

### Settings file

Everything on the Network page, and Save Format, is stored in `Config/net.txt`, which you can
also edit on a computer while slot is off:

```
wifi=on
syncthing=on
sync_in_game=off
tailscale=on
network_time=on
auto_timezone=on
timezone=America/Los_Angeles
save_format=retroarch

[network]
ssid=Home
psk=correct horse battery staple
```

Passwords are stored as plain text on the card, as BaseOS does with its SSH password.

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
| `crates/slot-net/` | **New.** Network thread, `wpa_supplicant` control, saved networks, Syncthing process and REST client. No dependencies. |
| `card/System/slotnet.sh`, `card/test-slotnet.sh` | **New.** Starts and stops `wpa_supplicant`/`udhcpc` on `wlan0`, with off-device tests. |
| `crates/slot-ui/src/net_screen.rs` | **New.** The Network pages and keyboard. |
| `crates/slot-ui/src/notice.rs`, `wifi_mark.rs` | **New.** Achievement notices; the Wi-Fi mark. |
| `crates/slot-ui/src/footer.rs` | 2 calls: the Wi-Fi mark beside both clocks. |
| `crates/slot-net/src/tailscale.rs` | **New.** Runs `tailscaled`, reads `tailscale status`, logs in and out. |
| `crates/slot-ach/` | **New.** RetroAchievements: rcheevos v12.5.0 (MIT, vendored), C glue, the offline layer, badges. |
| `crates/slot-retro/src/libretro.rs`, `core.rs` | Keeps the core's memory map (`SET_MEMORY_MAPS`) for rcheevos. |
| `crates/slot/src/emu.rs` | ~20 lines: an achievements session per game, run after each frame, reset after a state load or rewind. |
| `crates/slot/src/app/net.rs` | **New.** The Network phase inside the App. |
| `crates/slot/tests/network.rs`, `crates/slot-ui/tests/net_screen.rs` | **New.** Tests. |
| `crates/slot/src/app.rs` | ~25 lines: a `Network` phase, routing to `app/net.rs`, `shutdown_net()` before power off, restart and repair; load-newest by time, decoded state reads, no eviction undo for RetroArch slots; the Save Format row, hidden in game. |
| `crates/slot/src/frontend.rs` | `start_network`, `stop_network`, building text faces for the screen. |
| `crates/slot/src/link_radio.rs` | 3 lines: hold home Wi-Fi before a link, release after. |
| `crates/slot-ui/src/quick_menu.rs` | A `Network` row above About; `Save Format` at the end of Gameplay, with its two values. |
| `crates/slot-store/src/retroarch.rs` | **New.** The RetroArch layout, and reading RetroArch's rzip/`RASTATE` files. |
| `crates/slot-store/src/lib.rs` | Exports `StateRing` from `retroarch.rs`, which wraps upstream's ring. |
| `crates/slot/src/persist.rs` | GBA battery saves go to `Saves/mGBA/<game>.srm` under the RetroArch layout. |
| `crates/slot-net/src/time.rs` | **New.** Time zone lookup, daylight-saving offsets from the zone database, BaseOS's NTP switch. |
| `crates/slot/src/device_app.rs`, `host_app.rs` | Start the network at boot. |
| `taskfile.yml`, `.github/workflows/release.yml` | Fetch, check and ship Syncthing and Tailscale. |

## Taking new upstream releases

```sh
git fetch upstream --tags
git switch main && git merge --ff-only upstream/main   # main mirrors upstream, untouched
git switch net && git merge main                       # or: git merge v1.10.0
task test && task test:card
```

Conflicts, when they happen, are almost always in the lines listed above:

- **`QuickRow`**: keep `Network` and `SaveFormat` as the last variants of the enum and of `ALL`,
  and bump `ALL`'s length. The same for `QuickValue::RetroArch` and `SlotFormat`.
- **`app.rs`**: keep the `| QuickRow::Network` arms next to `QuickRow::About`, and the
  `| QuickRow::SaveFormat` arms in the same matches.
- **`emu.rs`**: keep `cheevos.frame()` right after the core runs a frame, `cheevos_stale` set
  wherever a state is loaded, and `drop(cheevos)` before the worker returns.

## Building

The same as upstream (`task dist:device`), plus `syncthing` and `tailscale` tasks that fetch the
pinned releases and check their checksums. `slot-ach` compiles rcheevos with the system C
compiler, which the device build's container already has. The host build (`cargo run`) shows a simulated radio with a few
made-up networks; the password for "Home Wi-Fi" is `password1`.

To run slot-net's tests against a real Syncthing, set `SLOT_TEST_SYNCTHING` to a Syncthing binary
for your computer:

```sh
SLOT_TEST_SYNCTHING=/path/to/syncthing cargo test -p slot-net
```
