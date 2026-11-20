# References

Read-only study, 2026-11-21. Nothing was cloned, built or run. Licenses were checked through the GitHub and crates.io APIs and the LICENSE files.

## Projects named in the brief

| Project | License | Stack | What to reuse | What to avoid |
|---|---|---|---|---|
| [Serein](https://github.com/ViceVerse-cz/Serein) | MIT OR Apache-2.0 | egui + wgpu, tokio, rusqlite, keyring | **Architecture**: a GUI-free `model` crate. The UI thread is the only owner of state and applies generation-tagged events. Network tasks send bounded messages and never mutate state. Every cache is bounded by bytes and item count. Also: thumbhash placeholders, skeletons, `request_repaint` only when an event arrives (~0.5 % idle CPU, ~145 MiB). | Nightly-ish toolchain pin, git-pinned egui, Discord-specific crates (TLS fingerprinting, voice, wry login). |
| [SteamFlow](https://github.com/weter11/SteamFlow) | MIT | Rust, egui, steam-vent | A `SteamProtocol` launch mode (`steam://rungameid`). Cover art tried in tiers (hero → library → header). | It logs into Steam itself, downloads depots and runs Wine directly. That's out of scope for us. |
| [Aurelia](https://github.com/Drackrath/Aurelia) | **GPL-3.0-or-later** (MIT up to commit 065782b) | Rust CLI (a SteamCMD alternative), Steam protobufs | **Ideas only**: CM protocol and depot logic as reference. | Its code (GPL). It edits Steam's files directly, and its README warns this can corrupt them. It also launches games outside Steam. |
| [Irori](https://github.com/irori-app/irori) | MIT | Rust, GPUI | **Best code to borrow**: `crates/store-steam/src/scan.rs` (libraryfolders → appmanifest `StateFlags` → localconfig playtime), the tool-app filter (Proton, Steam Linux Runtime, redistributables), and a file watcher instead of polling. Install, verify and uninstall all go through `steam://`. It also has an optional QR login for owned games. | GPUI (doesn't map to egui). The multi-store scope and 50-crate layout are too heavy for us. |
| [steam-vent](https://codeberg.org/steam-vent/steam-vent) | MIT | Rust Steam CM client 0.5.0 | Later, optional: PICS, owned apps and friends over the CM protocol. Protobufs are in `steam-vent-proto`. | Password login (against our rules). QR and refresh-token login aren't finished, so we'd write that flow ourselves. No high-level API. |
| [OpenSteamClient](https://github.com/OpenSteamClient/OpenSteamClient) | MIT | C#/Avalonia (the `c#-remake` branch; master is deprecated) | Proof that a Steam UI replacement is wanted. Its feature checklist. | It loads Valve's closed `steamclient.so` over IPC, which is fragile and closed. It replaces the client instead of sitting on top of it. |

## Other Rust apps with polished custom UIs

| App | License | Toolkit | What to learn |
|---|---|---|---|
| [Rerun viewer](https://github.com/rerun-io/rerun) | MIT OR Apache-2.0 | egui + wgpu | The largest polished egui app: a custom theme (`re_ui`), heavy virtualization and wgpu. Good patterns for theme tokens. |
| [Zed / GPUI](https://github.com/zed-industries/zed) | GPL-3.0 (editor), Apache-2.0 (gpui) | GPUI | `uniform_list` virtualization, fast startup, GPU text. Ideas only. |
| [Halloy](https://github.com/squidowl/halloy) | **GPL-3.0** | iced | TOML themes, low idle CPU, fast startup. Ideas only. |
| [Lapce](https://github.com/lapce/lapce) | Apache-2.0 | floem | A custom-drawn desktop UI with a plugin-free fast path. |

## Crates for Steam data

| Crate | License | Use |
|---|---|---|
| `keyvalues-parser` 0.2 | MIT OR Apache-2.0 | Text VDF: libraryfolders, appmanifest, localconfig, loginusers |
| `steam-vdf-parser` 0.1 | MIT OR Apache-2.0 | Binary `appinfo.vdf` v41. Our file is v41 (magic `0x07564429`). New crate, so test it on a real file and keep a minimal parser of our own as a fallback. |
| `new-vdf-parser` | LGPL-2.1 | Avoid (LGPL in a static binary) |
| `steamlocate` 2.1 | MIT | Finds Steam. We add our own path list: Debian `~/.steam/debian-installation`, native `~/.local/share/Steam`, Flatpak `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`, Snap. |
| `steamid-ng` 3 | MIT | SteamID conversion |
| `keyring` | MIT OR Apache-2.0 | Secret Service (gnome-keyring, KWallet) for tokens |

## UI assets

- **Font**: Steam uses the commercial Motiva Sans. The closest free match is **Figtree** (OFL-1.1, [repo](https://github.com/erikdkennedy/figtree)), with **Inter** (OFL-1.1) as the fallback for small text and Cyrillic. We bundle subset TTFs.
- **Icons**: **Phosphor** (MIT) through `egui-phosphor` (Apache-2.0). It has regular, bold, fill and thin weights, which suit Steam's thin outline icons. The alternatives were Lucide (ISC, no egui crate), Tabler (MIT) and Material Symbols (Apache-2.0, large font).

## Local facts (this laptop, read-only)

- The art cache is laid out per app: `appcache/librarycache/<appid>/{library_600x900.jpg, library_hero.jpg, library_hero_blur.jpg, logo.png, header.jpg}`, plus hashed files. The old flat layout was `<appid>_library_600x900.jpg`, so we support both.
- The CDN fallback is `https://shared.steamstatic.com/store_item_assets/steam/apps/<id>/<name>`. `cdn.cloudflare.steamstatic.com/steam/apps/<id>/<name>` also still works.
- Store data comes from `store.steampowered.com/api/appdetails?appids=<id>` (no auth). The full owned list needs an access token (`IPlayerService/GetOwnedGames`), so it's an optional login.

## Licence recommendation

Everything we'd borrow code from (Irori, Serein, SteamFlow, steam-vent, the crates) is MIT or Apache, and the GPL projects are used as ideas only. **I recommend `MIT OR Apache-2.0`**, the Rust convention and the same as Serein. GPL-3.0 is only needed if we decide to copy Aurelia or Halloy code. Pending the owner's decision.
