# Architecture

steam-rust is a native Steam client. It talks to Steam's servers directly (CM protocol and Web API), shows the library in an egui + wgpu window, and later downloads and launches games itself. This file is the map. Details live in each crate's docs.

## Bird's-eye view

```
 user input ──► ui ──Command──► client ──spawns──► steam (network, disk, keyring)
                ▲                 │  ▲                    │
                └── reads State ──┘  └──── Event ─────────┘
```

- **One owner of state.** `client::State` lives on the UI thread. Only the reducer (`Client::apply`) changes it.
- **Background work never touches state.** Tasks on the tokio runtime send an `Event` back through a channel, then wake the UI with `request_repaint()`.
- **The UI is a pure function of State.** Each frame it reads `State` and returns `Command`s. It never does I/O.

## Codemap

| Crate | Path | What it is |
|---|---|---|
| `sr-model` | `crates/model` | Plain data types: `AppId`, `Game`, `Account`, `InstallState`. No I/O and no dependencies besides serde. |
| `sr-steam` | `crates/steam` | Everything that talks to Steam or the disk: `auth` (QR login), `cm` (CM servers via steam-vent), `webapi`, `local` (reads an existing official Steam install, read-only), `vdf` (text KeyValues parser), `secrets` (keyring), `cache` (our own XDG files), `art` (cover-art lookup), `friends` (friends list, presence and 1:1 chat over the CM connection; no GUI types), `steam_client` (does a game ship the Steamworks library, is the official client running, start it minimized with `-silent`). |
| `sr-client` | `crates/client` | `State`, `Command`, `Event` and the reducer. Owns the tokio runtime and turns Commands into tasks. Modules: `downloads` (queue, one active download, progress events tagged with a job number), `play` (own launcher, running games, local play time; for Steamworks games it first starts the official client in the background when the setting allows, and never touches its files), `tools` (Proton manager), `manage` (uninstall, browse files), `settings`, `friends` (friends and chat state, the reconnecting session supervisor, demo friends; bounded: 2000 friends, 500 messages per chat). |
| `sr-ui` | `crates/ui` | egui views and widgets. `theme` holds every color, size and font as named tokens. |
| `steam-rust` | `crates/app` | The binary: eframe setup (wgpu), fonts, and wiring client and ui together. |
| `sr-cli` | `crates/steam/src/bin` | A developer CLI that exercises `sr-steam` without a GUI. |

## Architecture invariants

- **Dependency direction** is `model ← steam ← client ← ui ← app` (`ui` uses `client` only for `State`, `Command` and `Event`). Nothing depends upward.
- **`sr-model` has no I/O and no network crates.**
- **`sr-ui` never does I/O**, never holds tokens, and never `.await`s or blocks.
- **Secrets:** refresh tokens exist only in the system keyring and in memory. They never go to disk, logs or `Debug` output. Passwords are never handled at all, because login is QR-only.
- **We never edit the official client's configuration** (`config/*.vdf`, `libraryfolders.vdf`, its `appcache`). We only read it.
- **Game data is shared with Steam on purpose.** When we launch a Windows game that lives in a Steam library, it uses that library's `steamapps/compatdata/<appid>` prefix, so saves carry over both ways. Our own library uses Steam's on-disk format (`steamapps/appmanifest_<id>.acf`, `common/`), so the official client can adopt it as a library folder. Proton tools are installed into our own `compatibilitytools.d` unless the user picks Steam's.
- **Every async request carries a generation number.** The reducer drops results whose generation is stale, for example after logout or after switching accounts.
- **Every cache is bounded** by count and bytes: textures, decoded images and the HTTP art cache.

## Cross-cutting concerns

- **Idle CPU ≈ 0.** egui repaints only on input. Background tasks call `request_repaint()` once per batch of events, and animations use `request_repaint_after`.
- **Long lists are virtualized.** The library grid and list lay out only the visible rows (`ScrollArea::show_rows`).
- **Images** are decoded and downscaled on worker threads, uploaded once, and kept in an LRU of textures. Art comes from the official client's `appcache/librarycache` when it's present, otherwise from Steam's CDN into our cache. It is never committed.
- **Errors:** `anyhow` at the adapter level, plus typed enums wherever the UI has to react (for example `AuthExpired`). Every error is handled once, either propagated or shown.
- **Measuring:** `sr-cli` and the app print timings (`STEAM_RUST_TIMINGS=1`). Changes that affect performance note their before and after numbers in the commit message.

## Platforms

Linux (KDE Plasma, Wayland first) is the main target. Windows is supported from the start, so it never needs a rewrite. macOS is not a target, but nothing should rule it out.

- OS-specific code sits behind `cfg` in the smallest possible function. The rest of the crate calls that function and never branches on the OS itself. Current seams:
  - `secrets::platform_store`: Secret Service on Linux, Credential Manager on Windows.
  - `local::root_candidates`: XDG, Flatpak and Snap paths on Linux; the registry on Windows.
  - `store::private_dir`/`write_private`: Unix modes. On Windows, user-profile ACLs already keep these private.
  - Opening URLs.
  - Launching: Proton and umu on Linux; the exe runs directly on Windows.
- The in-game overlay (Shift+Tab) has its shortcut in `crates/app/src/hotkey.rs` (portal on Wayland, `XGrabKey` on X11, `RegisterHotKey` on Windows; registered only while `state.running` is non-empty and the setting is on) and its window in `sr-ui` (`overlay.rs`, an egui viewport). The hotkey thread only sends a toggle message and a repaint request; `sr-ui` stays free of I/O. Notes go through `Command::SetNote` to `notes.json`.
- Paths come from `directories` (XDG on Linux, `%APPDATA%`/`%LOCALAPPDATA%` on Windows) and never from hardcoded `~/.`.
- Linux-only features (Proton manager, layer-shell overlay) compile to nothing, or to a "not available" stub, on other systems.

## Checks

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` has to pass before every commit.
`cargo check --target x86_64-pc-windows-gnu --workspace` has to pass too. It needs `rustup target add x86_64-pc-windows-gnu` and the MinGW cross-compiler (`gcc-mingw-w64-x86-64`).
