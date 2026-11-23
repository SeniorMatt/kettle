# Architecture

Kettle is a native Steam client. It talks to Steam's servers directly (CM protocol and Web API), shows the library in an egui + wgpu window, and later downloads and launches games itself. This file is the map. Details live in each crate's docs.

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
| `sr-steam` | `crates/steam` | Everything that talks to Steam or the disk: `auth` (QR login), `cm` (CM servers via steam-vent), `webapi`, `local` (reads an existing official Steam install, read-only), `vdf` (text KeyValues parser), `secrets` (keyring), `cache` (our own XDG files), `art` (cover-art lookup), `store_more` (reviews, wishlist, tags and Browse lists: the public parts of the store beyond `storefront`, plus `storefront::set_locale` for the store's country and language; blocking, no login), `friends` (friends list, presence and 1:1 chat over the CM connection; no GUI types), `steam_client` (does a game ship the Steamworks library, with a per-install cache; is the official client running and is its IPC ready (pid file plus `registry.vdf` `ActiveProcess` or a fresh `/dev/shm/u<uid>-ValveIPCSharedObj-Steam`); start it minimized with `-silent` at low priority; the `Background` setting and its decision). |
| `sr-client` | `crates/client` | `State`, `Command`, `Event` and the reducer. Owns the tokio runtime and turns Commands into tasks. Modules: `downloads` (queue, one active download, progress events tagged with a job number), `play` (own launcher, running games, local play time; Steam is started quietly when Kettle opens if an installed game uses the Steamworks API (`steam_warm`, setting Automatic/Always/Never; it keeps running after Kettle quits), so Play finds it ready and waits only for its IPC, never for a fixed time; with "Run games without the Steam client" the game is pointed at our own steamclient (`steamroot`, the Steamworks host in `steamworks_host`, started with the session) and the official client is never started; Kettle never touches the official client's files or links), `tools` (Proton manager), `manage` (uninstall, browse files), `settings` (with `download_settings`: speed limit, schedule, update policy, notification toggles), `extras` (update checks every 45 minutes over the shared connection, install dialog data, library folders, game moves; one `Command::Manage` and one `Event::Extra`), `storage` (copy-then-delete game moves), `shortcuts` (`.desktop` files for `kettle --play <id>`), `notify` (desktop notifications), `store_more` (state and worker tasks for reviews, wishlist, tags and Browse; one `Command::StoreMore` and one `Event::StoreMore`; the country comes from the CM login, the language from the download setting; `open` allows only Valve's store and trailer CDN addresses), `friends` (friends and chat state, the reconnecting session supervisor, demo friends; bounded: 2000 friends, 500 messages per chat). |
| `sr-steamclient` | `crates/steamclient` | Kettle's own `steamclient.so` (cdylib + rlib): the C exports and C++ interface tables games load, and the socket protocol (`proto`, `ipc`) to Kettle. No Steam connection of its own; the host side is `client::steamworks_host`. See `docs/steamclient-design.md`. |
| `sr-ui` | `crates/ui` | egui views and widgets. `store` (with `store/extra.rs`: Browse, Wishlist, reviews, trailers) is the Store tab. `theme` holds every color, size and font as named tokens. |
| `kettle` | `crates/app` | The binary: eframe setup (wgpu), fonts, and wiring client and ui together. |
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

## Quality-of-life features (alpha 6)

Each lives in one `crates/client/src` file with its pure logic unit-tested, and one UI file in `crates/ui/src` that only draws and sends `Command`s.

| Feature | Client file | UI file | Command / state |
|---|---|---|---|
| Kettle updates itself | `kettle_update.rs` | `kettle_update_ui.rs` | `Command::KettleUpdate`, `State::kettle_update`, `Settings::check_kettle_updates` |
| Start with the computer | `autostart.rs` | `settings_extra.rs` | `Command::SetAutostart`, `State::autostart` (the file `~/.config/autostart/kettle.desktop` is the truth) |
| Friend notifications | `friend_notify.rs` (rate limits) + `friends.rs` | existing Settings > Notifications | `NotifySettings::{friend_online, friend_game}` |
| ProtonDB rating | `protondb.rs` | `protondb_badge.rs` | `Command::ProtonDb`, `State::protondb` |
| Per-game environment | `game_env.rs` (applied in `play.rs`) | `game_env_ui.rs` | `Command::SetGameEnv`, `State::game_env`; file `game_env.json` |
| Non-Steam games | `non_steam.rs`, `sr_steam::run::prepare_exe` | `add_game_ui.rs` | `Command::NonSteam`, `State::non_steam`; file `non_steam.json` |

- **Self-update** only acts when `$APPIMAGE` is set. It reads GitHub's release list (pre-releases included, a 404 is silent), picks exactly `kettle-x86_64.AppImage` from `github.com/SeniorMatt/kettle/releases/download/`, downloads to `<AppImage>.update.tmp` in the same folder, checks size and the ELF header, `chmod +x`, and renames over the running file (atomic). The user clicks "Update now" first and "Restart now" after.
- **Non-Steam games** get ids with the top bit set (Steam's own shortcut formula, CRC-32 of the quoted program and the name), so they never clash with app ids. They are merged into `State::games` by `merge_installed` and use the normal play-time bookkeeping. `play.rs` hands them to `non_steam::play_non_steam`, which asks `run::prepare_exe` for a plan: a Linux program as it is, a Windows one through the default Proton tool with a prefix in `<data dir>/compatdata/<id>`. Steam's `shortcuts.vdf` is parsed read-only for the import.
- **Environment variables** are validated (`[A-Za-z_][A-Za-z0-9_]*`, no line breaks or NUL, at most 64 per game) and added to the launch plan after everything else, so they win. The launch log lists them.

## Cross-cutting concerns

- **Idle CPU ≈ 0.** egui repaints only on input. Background tasks call `request_repaint()` once per batch of events, and animations use `request_repaint_after`.
- **Long lists are virtualized.** The library grid and list lay out only the visible rows (`ScrollArea::show_rows`).
- **Images** are decoded and downscaled on worker threads, uploaded once, and kept in an LRU of textures. Art comes from the official client's `appcache/librarycache` when it's present, otherwise from Steam's CDN into our cache. It is never committed.
- **Errors:** `anyhow` at the adapter level, plus typed enums wherever the UI has to react (for example `AuthExpired`). Every error is handled once, either propagated or shown.
- **Measuring:** `sr-cli` and the app print timings (`KETTLE_TIMINGS=1`). Changes that affect performance note their before and after numbers in the commit message.

## Platforms

Linux (KDE Plasma, Wayland first) is the main target. Windows is supported from the start, so it never needs a rewrite. macOS is not a target, but nothing should rule it out.

- OS-specific code sits behind `cfg` in the smallest possible function. The rest of the crate calls that function and never branches on the OS itself. Current seams:
  - `secrets::platform_store`: Secret Service on Linux, Credential Manager on Windows.
  - `local::root_candidates`: XDG, Flatpak and Snap paths on Linux; the registry on Windows.
  - `store::private_dir`/`write_private`: Unix modes. On Windows, user-profile ACLs already keep these private.
  - Opening URLs.
  - Launching: Proton and umu on Linux; the exe runs directly on Windows.
  - Launch logic follows the official client (`run.rs`, `compat.rs`): entry = lowest id for the OS, a Linux entry must name Linux (an entry with no `oslist` that runs an `.exe` is a Windows one: Fallout 2); exe and working dir are matched case-insensitively; tool order = our per-game, Steam's per-game (`config.vdf`), Valve's per-game (`appinfo` app 891390 `app_mappings`), our default, Steam's global default, Valve's `proton-stable`, newest. Proton runs in the Steam Linux Runtime named by the tool's `require_tool_appid` (`run::runtime_for` matches the runtime's folder against `installdir` of that app's manifest, never "any runtime in the library"; `doctor.rs` re-checks every file a plan needs: `sr-cli launch <id> --doctor`, and the short form is appended to a failed launch's details), with `STEAM_COMPAT_*` (incl. library paths, shader path, `STEAM_COMPAT_CONFIG` and env from `app_compat_configs`). Native games start through `<steam>/ubuntu12_32/steam-runtime/run.sh`, or in the container Valve maps them to. Flatpak Steam: its tools, runtimes and prefixes are plain folders under `~/.var/app/com.valvesoftware.Steam`, so we run them from the host (the container needs no Flatpak sandbox) with the Flatpak root as the client path and the same `compatdata`; Steamworks games that need the client running use "Launch via Steam client". Every launch writes `logs/launch-<appid>.log` (command, variables, output).
- The in-game overlay (Shift+Tab) is drawn inside the game's own frames by `libkettle_overlay.so` (see "In-game overlay library" below); `sr-ui` keeps an in-process window (`overlay.rs`) as the fallback for games without the library (Gamescope, Windows) and `sr-ui` stays free of I/O. Notes go through `Command::SetNote` to `notes.json`. Games can run inside Gamescope (`sr_steam::gamescope`: settings, per-game override in `gamescope_games.json`, the command `gamescope <opts> -- sh -c <shim> ... <full plan command>` built last in `run::prepare_with`, outside the container, Proton and the user's wrappers). The shim writes `$DISPLAY` (Gamescope's private X server) to `$XDG_RUNTIME_DIR/kettle-gamescope-<appid>.display`; Inside Gamescope the overlay library is not loaded (its environment would reach Gamescope itself), so the in-process window is used there. The FPS counter is drawn by the overlay library when it is loaded; inside Gamescope it is MangoHud (`sr_steam::mangohud` writes its config and the launch environment).
- Paths come from `directories` (XDG on Linux, `%APPDATA%`/`%LOCALAPPDATA%` on Windows) and never from hardcoded `~/.`.
- Linux-only features (Proton manager, layer-shell overlay) compile to nothing, or to a "not available" stub, on other systems.

## Checks

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` has to pass before every commit.
`cargo check --target x86_64-pc-windows-gnu --workspace` has to pass too. It needs `rustup target add x86_64-pc-windows-gnu` and the MinGW cross-compiler (`gcc-mingw-w64-x86-64`).

## Steam Cloud saves

- `sr_steam::cloud` is pure: reads the `ufs` section of appinfo (savefiles, rootoverrides), maps roots to folders (native Linux: `gameinstall`, `LinuxHome`, XDG dirs; Proton/umu: the Windows roots inside `compatdata/<app>/pfx/drive_c/users/steamuser`; `remote/` of `userdata/<account>/<app>` for `ISteamRemoteStorage`, only in a Kettle-made root: skipped when an official Steam root exists), scans local files (SHA-1), keeps a per-app `SyncState` (`<data dir>/cloud/<appid>.json`) and decides: download / upload / conflict. It never deletes anything.
- `sr_steam::cloud_sync` runs the plan through a `Backend` trait: `before_launch` (download newer files, check SHA-1, copy replaced files to `cloud-backups/`, state saved only on success) and `after_exit` (upload changed files in one batch; nothing if the cloud changed too). `CmBackend` speaks `Cloud.GetAppFileChangelist`, `ClientFileDownload`, `BeginAppUploadBatch`, `ClientBeginFileUpload`, `ClientCommitFileUpload` and `CompleteAppUploadBatchBlocking` over the shared CM connection, and moves bytes over HTTPS to the hosts Steam names. Written against the real proto types, not yet tried on a real account.
- `sr_client::cloud` wires it in: the sync before a launch runs on the launch worker (`play::launch`) before the game or the Steam client starts; a conflict emits `CloudEvent::Conflict` and stops the launch, `Command::CloudResolve` answers and starts it again. A failed sync still lets the game start but leaves no context, so nothing is uploaded after it. Settings: `Settings::cloud_sync` (default on) and `cloud_games.json` (per-game off). `KETTLE_CLOUD_DRYRUN=1` logs the plan only. Everything is written to the launch log.
- UI hooks are in `crates/ui/src/cloud_ui.rs` (`conflict_dialog`, `settings_row`, `game_row`, `status`).

## In-game overlay library

Steam's overlay is drawn inside the game's frames and takes the game's input while open. Kettle does the same
with `crates/overlay-hook` (package `sr-overlay-hook`, cdylib `libkettle_overlay.so`), loaded two ways that
`KETTLE_OVERLAY=1` switches on for games Kettle starts:

- `LD_PRELOAD` (appended to the game's own value; both `libkettle_overlay.so` and the i686 `libkettle_overlay32.so`, the
  loader skips the one of the wrong word size with a harmless message on stderr): hooks
  `glXSwapBuffers` / `eglSwapBuffers*` (also found through `glXGetProcAddress*`, `eglGetProcAddress` and a hooked
  `dlsym`, which is how SDL loads them), SDL2 / SDL3 event and mouse functions, and Xlib event and grab functions.
- Vulkan implicit layer `VK_LAYER_KETTLE_overlay` (manifest `kettle_overlay.json` in
  `$XDG_DATA_HOME/vulkan/implicit_layer.d`, which the Steam Linux Runtime container imports like MangoHud's):
  draws before `vkQueuePresentKHR`. DXVK and VKD3D are ordinary Vulkan applications, so Proton games are covered.
  The 32-bit copy has its own manifest (`kettle_overlay_32.json`, same layer name).
- Inside Gamescope the library goes into the game, not into Gamescope: `gamescope::apply` puts `env LD_PRELOAD=...
  KETTLE_OVERLAY=... KETTLE_OVERLAY_SOCKET=...` right in front of the game's command (behind its display shim), so
  Shift+Tab works there too.

Modules (`src/`): `state` (the one overlay of the process: egui context, data, frame timing), `input` (pure input
state machine: Shift+Tab toggles, everything is swallowed while open, releases of keys the game saw pressed still
reach it), `sdl` and `xlib` (feed it, and release relative mode, grabs and the hidden cursor on open; the game's own
calls are remembered while open and applied again on close), `gl` (egui_glow; saves and restores all GL state it
touches), `vk` (own small egui renderer, shader compiled from `overlay.wgsl` by `build.rs`; submits one command
buffer on the present queue that waits for the game's semaphores and signals its own), `real` (real function lookup,
`dlsym` hook), `guard` (see below), `shot` (screenshots), `ipc` (socket to Kettle). Programs that create windows through SDL are handled by the SDL hooks;
the Xlib hooks (Wine, GLFW) stay out of their way. The look is `crates/overlay-ui`; the wire types are in
`crates/overlay-proto` (JSON lines over `$XDG_RUNTIME_DIR/kettle-overlay-<pid>.sock`, named to the game in
`KETTLE_OVERLAY_SOCKET`). Kettle's side is `crates/app/src/overlay_host.rs`; it installs the library to
`<data dir>/overlay/` at start (`overlay_install.rs`, `sr_steam::overlay_hook`) and `run.rs` adds the variables.
**A bug in the overlay never kills the game.** Every function the library exports to the game runs its own code
through `guard::run` (`catch_unwind`; the `hooks!` macro does it for every GL, SDL, Xlib, XInput2 and `dlsym` hook, the
Vulkan layer entries call it by hand). A panic is caught, logged once, the overlay switches itself off for the process
(closed, grabs and cursor given back) and the call goes straight through to the real function. This needs unwinding
panics, so the workspace profiles have no `panic = "abort"` (Cargo ignores a per-package setting); the test
`guard::tests::panics_are_caught` aborts if that is ever put back. The Vulkan layer keeps the next layer's lookup
functions in plain maps filled first, so the game's own lookups work whatever happens to the rest of our bookkeeping.
A lost Kettle: sends never block (`MSG_DONTWAIT | MSG_NOSIGNAL`, a hung or stopped Kettle cannot stall a frame or raise
SIGPIPE in the game), the connect loop retries every 2 s, and the overlay keeps showing the last data it got.

**Data and actions.** Kettle sends `OverlayData` (game, session, total playtime, achievements from the game page's
data, friends, the screenshot folder) every 0.5 s while the overlay is open and every 3 s otherwise, and `avatar`
messages (48 px RGBA, base64, once per connection) for the account and the friends it lists. `OpenUrl` (allow-listed in
`sr_client::open_url`) and `OpenChat` are carried out in Kettle. F12 and the overlay's button take a screenshot: the GL
hook reads the back buffer with `glReadPixels`, the Vulkan layer copies the swapchain image (made with the
transfer-source usage) before the overlay is drawn into it; a thread writes a PNG to `~/Pictures/Kettle/<game>/` and
sends `screenshot{path}` to Kettle. Hi-dpi: the surface is `scale` times the window's size in points (SDL's
`SDL_GetWindowSizeInPixels` over `SDL_GetWindowSize`); egui gets `native_pixels_per_point = scale` and a screen in
points, which is what the game's mouse coordinates use. XInput2: `XIGrabDevice` / `XIUngrabDevice` are virtualized like
`XGrabPointer`, and the raw-input bits (13..17) of the game's `XISelectEvents` are taken away while the overlay is open.
Xlib event offsets depend on the pointer width (`xlib::off`), so the i686 build reads 32-bit `XEvent`s correctly.

Proof: `scripts/overlay-test.sh` (SDL2 test program under Xvfb), `overlay-gl-vk-test.sh` (glxgears, vkcube),
`overlay-ddnet-test.sh`, `overlay-kettle-test.sh` (a fake Kettle on the real socket), and the checks in
`.github/workflows/check.yml`.
