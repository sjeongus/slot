# slot, with Wi-Fi and Syncthing

This is a fork of [BrandonKowalski/slot](https://github.com/BrandonKowalski/slot). It adds a
**Settings > Network** page. Use it to join Wi-Fi, typing the password with the D-pad, and to keep
`Saves/` and `States/` in step with your other devices through
[Syncthing](https://syncthing.net). Everything else is upstream slot, and new upstream releases
are merged in as they come.

## Using it

1. Open the menu on the shelf (MENU) and pick **Network**.
2. **Network**: press A to scan, choose your network, and type its password.
   - A types the highlighted key; B deletes; Y switches capitals; SELECT shows symbols; START
     joins.
   - Saved networks are joined automatically every time the device is on.
   - **Other Network…** at the bottom of the list joins a hidden network.
3. **Syncthing**: turn it on. Once the device is on Wi-Fi, **Syncthing Details** shows:
   - this device's ID, as text and as a QR code you can scan from the Syncthing app on a phone;
   - the address, user and password of the Syncthing web interface.
4. On your computer, add the handheld as a device in Syncthing using that ID. Then accept the two
   folders it offers:

   | Folder ID     | On the card | What                                                        |
   |---------------|-------------|-------------------------------------------------------------|
   | `slot-saves`  | `Saves/`    | battery saves (`.sav`/`.srm`); 5 old versions kept on the card |
   | `slot-states` | `States/`   | save state rings                                            |

   Save states only load in the same emulator (mGBA or gpSP) at the same version, so share
   `slot-states` only with other slot devices.

5. **Sync In Games** is off by default. While a cart is in, Syncthing pauses both folders, so a
   save from another device can't land under a running game, and the emulator keeps the CPU.
   Ejecting resumes them and rescans.

### Link play

Linking needs the radio to itself. When you start a link, slot turns its own Wi-Fi and Syncthing
off. When the link ends, it turns them back on and rejoins your network. Nothing to do by hand.

On models whose BaseOS build has no Wi-Fi (the RG28XX), the Network row is hidden.

### Settings file

Everything on the Network page is stored in `Config/net.txt`, which you can also edit on a
computer while slot is off:

```
wifi=on
syncthing=on
sync_in_game=off

[network]
ssid=Home
psk=correct horse battery staple
```

Passwords are stored as plain text on the card, as BaseOS does with its SSH password.

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
| `crates/slot/src/app/net.rs` | **New.** The Network phase inside the App. |
| `crates/slot/tests/network.rs`, `crates/slot-ui/tests/net_screen.rs` | **New.** Tests. |
| `crates/slot/src/app.rs` | ~16 lines: a `Network` phase, routing to `app/net.rs`, `shutdown_net()` before power off, restart and repair. |
| `crates/slot/src/frontend.rs` | `start_network`, `stop_network`, building text faces for the screen. |
| `crates/slot/src/link_radio.rs` | 3 lines: hold home Wi-Fi before a link, release after. |
| `crates/slot-ui/src/quick_menu.rs` | A `Network` row above About. |
| `crates/slot/src/device_app.rs`, `host_app.rs` | Start the network at boot. |
| `taskfile.yml`, `.github/workflows/release.yml` | Fetch, check and ship Syncthing. |

## Taking new upstream releases

```sh
git fetch upstream --tags
git switch main && git merge --ff-only upstream/main   # main mirrors upstream, untouched
git switch net && git merge main                       # or: git merge v1.10.0
task test && task test:card
```

Conflicts, when they happen, are almost always in the lines listed above:

- **`QuickRow`**: keep `Network` as the last variant of the enum and of `ALL`, and bump `ALL`'s
  length.
- **`app.rs`**: keep the `| QuickRow::Network` arms next to `QuickRow::About`.

## Building

The same as upstream (`task dist:device`), plus a `syncthing` task that fetches the pinned
release and checks its checksum. The host build (`cargo run`) shows a simulated radio with a few
made-up networks; the password for "Home Wi-Fi" is `password1`.

To run slot-net's tests against a real Syncthing, set `SLOT_TEST_SYNCTHING` to a Syncthing binary
for your computer:

```sh
SLOT_TEST_SYNCTHING=/path/to/syncthing cargo test -p slot-net
```
