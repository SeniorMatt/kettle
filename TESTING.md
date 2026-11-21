# Manual test checklist

Things the automated checks can't cover: a real desktop, a real account and real games. Run them on a real machine (Linux first), top to bottom. Each step says what to do and what you should see.

## 0. Get the build

- AppImage: copy `dist/kettle-x86_64.AppImage` to the machine, then `chmod +x kettle-x86_64.AppImage && ./kettle-x86_64.AppImage`.
- Or from source: `cargo run --release -p kettle`.
- Timings: start it with `KETTLE_TIMINGS=1` and note the `[timing]` lines printed in the terminal.

## 1. Window

- [ ] The window opens in under a second (warm start), with the custom title bar.
- [ ] Drag the title bar: the window moves. Double-click it: it maximizes and restores.
- [ ] Hold **Alt** and drag anywhere in the window: it moves. Note: KDE may take Alt+drag itself. If it does, System Settings → Window Management → Window Behavior → Window Actions → Modifier key can be set to Meta.
- [ ] Drag each edge and corner: the window resizes.
- [ ] Minimize, maximize and close work.
- [ ] **Ctrl+Tab** cycles Store → Library → Community.

## 2. Sign in

- [ ] The QR code shows. Scan it with the Steam Mobile App (Steam Guard → Scan a QR code) and approve.
- [ ] The library fills with your games and the covers fade in.
- [ ] Close and reopen the app: you're still signed in and the library shows instantly.
- [ ] Look in the keyring (KWallet Manager or Seahorse): there's one `kettle` entry. Check `~/.local/share/kettle/` too: no token in any file.

## 3. Library

- [ ] Games installed by the official client show as installed, with their size.
- [ ] Scrolling the grid stays smooth. Hover lifts a cover a little.
- [ ] Sidebar search filters as you type.
- [ ] Sorting (Alphabetical / Last Played / Hours Played) works.
- [ ] With the window idle, CPU is about 0% in `top`.
- [ ] Game page (written, not yet tried on a real account): below the tab box, news shows a FEATURED card, ACTIVITY grouped by day and "View Latest News"; a click opens the item on store.steampowered.com. A game you have run in the official client shows ACHIEVEMENTS ("You've unlocked X/Y"), matching Steam's own page; a game with DLC shows ADDITIONAL CONTENT; a friend playing the game shows under FRIENDS WHO PLAY. A game with none of these shows no empty boxes.

## 4. Downloads

Pick a small game you own (under 2 GB).

- [ ] On its page, INSTALL → the dialog shows the libraries (ours and Steam's) and the free space.
- [ ] Install into the kettle library. The Downloads page and the bottom bar show the speed and percentage.
- [ ] Pause, wait, then Resume: it continues from where it stopped.
- [ ] When it finishes, the game page shows PLAY and the size on disk.
- [ ] Gear → Verify files: it finishes with nothing re-downloaded.
- [ ] Gear → Browse local files: the game's folder opens.

### Install dialog, updates and the Downloads page (written, not yet tried)

- [ ] INSTALL dialog: "Required" and "Available" space show, a game with several languages has a language box, DLC are listed with sizes. With a small disk or a huge game, Install is disabled with a red note.
- [ ] Tick "Create desktop shortcut" and "Create application menu shortcut", install. After it finishes the icons exist (`~/.local/share/applications/kettle-game-<id>.desktop`, `~/Desktop/...`). Double-click one: Kettle starts and the game launches (`kettle --play <id>`). Uninstall removes both.
- [ ] Downloads page: the running game shows big art, "DOWNLOADING n%", a graph of the last 60 s (blue network, green disk), speed, peak, downloaded, progress and time left. Queue two more games: UP NEXT shows arrows to move them, a pause and a remove button. Pause all, Resume all. COMPLETED has "Clear".
- [ ] Update a game: install an older build or wait for a real update. Settings > Downloads > "Check now": the game shows "Update available" and the game page offers UPDATE. Updating downloads only a fraction of the game (see "DOWNLOADED" on the Downloads page).
- [ ] Press PLAY on a game with an update waiting: "Update required" with Update now / Play anyway.
- [ ] A game installed by the official Steam client with an update: UPDATE asks first ("Update this time / Always allow / Never"). With "Never" nothing is written to Steam's folder. Check with `ls -l` that its files are untouched.
- [ ] Per game: Properties > General > Automatic updates: "Only update when I launch it" stops automatic downloads for that game.
- [ ] Settings > Downloads > "Limit download speed" at 2048 KB/s: the Downloads page stays near 2 MB/s. Switch it off: speed returns. "Display download rates in bits per second" shows Mbps.
- [ ] Schedule: set "Only update between set hours" to a window that does not include now and press "Check now": updates are listed but not downloaded. Set it to include now: they start.
- [ ] "Allow downloads during gameplay" off: start a game from Kettle during a download; the page says it waits; close the game: it continues.

### Storage, library folders, notifications (written, not yet tried)

- [ ] Settings > Library: add a folder (an absolute path); it appears with free space and as an install target. Remove it again (works only when no game is in it).
- [ ] Settings > Storage: each library shows used and free space and its games, biggest first. Move a game to another library: progress bar, the game still plays, the old folder is gone. Cancel a move halfway: the original is untouched and no `.kettle-move-*` folder is left behind.
- [ ] Moving a Steam-library game needs the extra "Move (touches Steam's folder)" click.
- [ ] Settings > Notifications: "Send a test" pops up a desktop notification; finishing a download shows one when enabled.
- [ ] Settings > Account: "Keep me signed in" off, restart: the QR login shows. "Delete local data" clears the cache folder; covers load again.

## 5. Play

- [ ] A native Linux game: PLAY → it starts and the button shows RUNNING. Quit the game: play time goes up.
- [ ] A Windows game already installed by Steam: PLAY → it starts through Proton, with the same saves as in Steam.
- [ ] Fallout 2 (38410) installed by Steam (also Fallout 3, Mafia, Magicka): PLAY starts it through Proton. `sr-cli launch 38410 --dry-run` shows `entry: id 0 ... fallout2launcher.exe`, `exe on disk: yes`, the prefix state and the container line; compare env and command with Steam's own (`ps`/`/proc/<pid>/environ` of the running game).
- [ ] With Flatpak Steam only: PLAY a Proton game from the Flatpak library; saves are the ones Steam shows. If it fails, the dialog shows the reason, "Copy details" text and `~/.local/share/kettle/logs/launch-<appid>.log`.
- [ ] Gear → Compatibility…: pick another Proton; the game uses it the next time.
- [ ] If a game won't start, check the log path in the message (`~/.local/share/kettle/logs/`). Then try Gear → Launch via Steam client.
- [ ] Steamworks games: with Settings → Games → "Start the Steam client in the background" on, PLAY on a game that ships `libsteam_api.so` / `steam_api64.dll` shows STARTING STEAM… and then starts the official client minimized (`-silent`) and the game. With it off, or without an installed client, the game starts anyway; if it exits within 10 s a notice says it needs the Steam client. `sr-cli launch <appid> --dry-run` prints "needs the Steam client: yes/no, running: yes/no" and starts nothing.

### In-game overlay (Shift+Tab)

- [ ] KDE Wayland: start a game from the launcher. KDE shows a one-time "kettle wants to register a shortcut" prompt; accept it. System Settings → Shortcuts lists kettle "Show or hide the in-game overlay" with Shift+Tab.
- [ ] With the game in focus (also fullscreen), Shift+Tab shows the dimmed overlay over it with a fade-in: game name, session time, clock, friends and chat, Notes, "Return to game".
- [ ] Shift+Tab, Esc or "Return to game" hides it and the game has focus again (keyboard and mouse work).
- [ ] Type a note, close, press Shift+Tab again: the note is still there. Restart the app: it is still there.
- [ ] With no game running, Shift+Tab does nothing here: other programs get it (e.g. Shift+Tab moves focus backwards in a browser). Quit the game: same.
- [ ] Settings → In Game → "Enable the in-game overlay" off: Shift+Tab is no longer taken and Games → "Show overlay" is gone.
- [ ] X11 session: the same checks (the shortcut is a key grab, no prompt).
- [ ] Windows: same checks with a fullscreen-windowed game. If the overlay shows behind an exclusive-fullscreen game, note it.
- [ ] While the overlay is hidden the app uses no CPU (`top`).

#### Overlay helper window (no window behaviour)
The overlay is a second process, `kettle --overlay <appid> --socket <path>`, an override-redirect X11 window (XWayland on Wayland). Check with a game that runs under Proton (an XWayland window):
- [ ] KDE Wayland: `pgrep -a kettle` shows the helper while the game runs, and it is gone a moment after the game exits. Games > "Show overlay" and Shift+Tab both show it.
- [ ] No open or close animation of the desktop (no zoom, fade or minimize effect), no taskbar or task switcher entry, no new window in the KDE window list (`qdbus org.kde.KWin /KWin org.kde.KWin.showDesktop` or Alt+Tab).
- [ ] The game is not minimized and not un-fullscreened when the overlay opens or closes, also for a fullscreen game (borderless and, if it works, exclusive). Repeat 20 times in a row (Shift+Tab, Shift+Tab, ...): no flicker and no double show (the 300 ms debounce).
- [ ] While shown: typing in a note and in a chat does not reach the game (no WASD movement). After hiding, the game gets keyboard input at once.
- [ ] Mouse: clicks hit the overlay buttons. A game that captures the mouse (FPS) may keep the pointer; note which games do.
- [ ] The overlay covers the monitor the game is on (it uses the primary monitor; note if the game is on another one).
- [ ] While hidden, the helper uses no CPU (`top -p $(pgrep -f 'kettle --overlay')`), and the compositor still unredirects a fullscreen game (nothing is mapped).
- [ ] X11 session: same checks; Shift+Tab comes through the helper's keyboard grab, one press toggles once.
- [ ] No `XDG_RUNTIME_DIR` or no `DISPLAY` (run the app with it unset): the built-in overlay window is used and a line `[overlay] ...; using the built-in overlay window` is printed.
- [ ] `ls -l $XDG_RUNTIME_DIR/kettle-overlay-*.sock` shows mode `srw-------` while a game runs and no file after it exits or the app is closed.
- [ ] Top bar shows game, session time, clock, Friends, Settings, Return to game. Friends: online and in-game friends first, click opens a chat, a sent message appears. Notes, quick settings (overlay on/off, performance overlay on/off, preset) work. Controls are about 40 px high.
- [ ] Settings > Interface > Animations = Off: the overlay appears without a fade.

#### Gamescope (Settings > Run games in Gamescope; `sr-cli launch <appid> --dry-run --gamescope`)
Needs `gamescope` installed (`sudo apt install gamescope`). Never tried in a real session yet; test on KDE Wayland first.
- [ ] Not installed: the settings section shows the apt, pacman and dnf lines; with the switch on a game still starts normally and the terminal says `[gamescope] not installed`.
- [ ] Installed: the section shows the version. `sr-cli launch <appid> --dry-run --gamescope` prints `gamescope: yes` and `command: gamescope -f -- /bin/sh -c '...' kettle-gamescope <run dir>/kettle-gamescope-<id>.display <usual command>`; the user's `gamemoderun` is right after the display file, the container / `proton` after it.
- [ ] KDE Wayland, fullscreen: the game opens in a fullscreen Gamescope window on the monitor; Alt+Tab to another window and back does not minimize or un-fullscreen the game; other windows (notifications, the app) cannot cover it.
- [ ] Window / Borderless / Windowed, game resolution lower than the window with FSR, NIS and Nearest: the picture is scaled. Sharpness changes the FSR look.
- [ ] Frame rate limit: the game is capped (turn the performance overlay on to see it). With the cap only on the performance overlay, it still caps (through Gamescope) and does not cap twice.
- [ ] Performance overlay on with `mangoapp` installed: one HUD, drawn by Gamescope; `MANGOHUD=1` is not in the command. Without `mangoapp`: the usual in-game HUD.
- [ ] Overlay: `ls $XDG_RUNTIME_DIR/kettle-gamescope-*.display` shows `:N` a few seconds after the start. Shift+Tab draws the overlay over the game without changing the game's focus; keys type into chat; mouse clicks hit the buttons; closing returns input to the game. Repeat 20 times. If the overlay never shows, the terminal says `[overlay] the game's Gamescope display never appeared` after 90 s and the built-in window is used.
- [ ] Mouse: a game that loses the mouse works with "Always grab the mouse" on. Note which games need it.
- [ ] HDR only with the switch on and an HDR screen. Adaptive sync only with the switch on.
- [ ] NVIDIA: Gamescope needs a recent driver and `nvidia-drm.modeset=1`; note any black window, missing cursor or crash and the driver version.
- [ ] Per-game: Properties > Gamescope "Never" with the global switch on starts the game without it; "Always" with it off starts with it. The choice is in `gamescope_games.json`.
- [ ] A game's own launch options `gamescope -f -- %command%` are respected: we add no second Gamescope.
- [ ] Stop the game from the app: Gamescope and the game end together (`pgrep gamescope` empty).

#### Performance overlay (MangoHud)
- [ ] Without MangoHud installed: Settings > In Game > Performance overlay says so with apt, pacman and dnf commands; the overlay's Settings panel says so; games start normally.
- [ ] With MangoHud (`sudo apt install mangohud`): `sr-cli launch <appid> --dry-run --hud fps` prints `env: MANGOHUD=1` and `env: MANGOHUD_CONFIGFILE=<data dir>/mangohud/<appid>.conf`, and for a native game the program is `mangohud` with the game as first argument. The file is mode 0600 and lists every option as `key=0|1`.
- [ ] Launch a Proton game with each preset (FPS only, FPS + graph, Full, Custom): the HUD shows what the preset names, at the chosen position, size, colour and opacity. Shift_L+F12 hides and shows it in the game.
- [ ] Frame rate limit 30 / 60 / 120 / 144 / custom is respected in the game.
- [ ] Launch options with their own `mangohud %command%` or `MANGOHUD_CONFIG=...`: the app adds nothing (one HUD, theirs).
- [ ] Native OpenGL game: HUD shows (wrapper). Native Vulkan game: HUD shows.
- [ ] Changing the preset from the in-game overlay saves it (Settings > In Game shows it) and applies at the next launch.

## 5b. Collections, favourites and filters

- [ ] With collections made in the official Steam client, the sidebar shows FAVORITES, one section per collection and ALL; the same games are in each.
- [ ] Right-click a game: Add to Favorites. It appears under FAVORITES at once; Remove from Favorites takes it out (also when Steam had it as a favourite). Steam's own files are not changed.
- [ ] Hide this game: it leaves every list and shows only under HIDDEN (folded at first). Unhide brings it back.
- [ ] Add to, New collection...: type a name, Create. The game is in the new section. Restart the app: it is still there. Right-click the section header, Delete collection.
- [ ] Click a section header to fold it; click the squares icon to show that collection in the library grid. Home goes back to all games.
- [ ] GAMES drop-down: Installed only and Ready to play narrow every section. Search narrows all sections and hides empty ones.
- [ ] Library page: SHOW (All, Installed, Not installed) next to SORT BY. With 2000+ games scrolling stays smooth and idle CPU is about zero.
- [ ] A dynamic Steam collection is listed with a note and only its hand-added games.

### Game properties

- [ ] Right-click a game, or open its gear menu, and choose "Properties...": a window titled with the game's name opens with General, Compatibility and Installed Files (the last one only for installed games). Esc and the close button close it; the main window keeps working.
- [ ] General: the launch options field shows what the official client has for this game, if anything. Type `-novid`, press Enter, close and reopen: it is still there. Clearing the text and pressing Enter stays empty after a restart (the official client's value no longer applies).
- [ ] Set `FOO=1 gamemoderun %command% -novid`, then run `sr-cli launch <appid> --dry-run`: the "command:" line starts with `FOO=1 gamemoderun`, then the game, and ends with `-novid`. The same text without `%command%` only appends arguments.
- [ ] Play the game with `MANGOHUD=1 %command%` (or another harmless variable) and check the log in `~/.local/share/kettle/logs/` or the game's behaviour. Windows game through Proton: wrappers run before Proton.
- [ ] Compatibility: turn on "Force the use of a specific compatibility tool", pick one, close and reopen: it is kept. Turn it off: the official client's choice is used again. "Compatibility..." in the game's menu opens this tab.
- [ ] Installed Files: Browse... opens the folder, Verify queues a verify and shows Downloads, Uninstall... asks "Keep / Uninstall" in place. Buttons are dimmed while the game runs or downloads.
- [ ] The window uses no CPU while idle.

## 6. Proton manager

- [ ] Settings → Compatibility lists Steam's Protons and any custom ones.
- [ ] Install the latest GE-Proton: the progress bar moves and it then shows as installed.
- [ ] Use it for one game, play, and then Remove it.

## 7. Store

- [ ] Store tab: the carousel, specials and search all work.
- [ ] Open a game, then step through its screenshots. Add to cart opens your browser.
- [ ] A store page shows "Very Positive (12,345)"-style review summary and the ten most helpful reviews below the buy box.
- [ ] A store page with trailers shows thumbnails under the screenshots; a click opens the video in your video player or browser.
- [ ] Store > Wishlist (signed in, profile game details public): your games in your own order with price and discount; Refresh works; the browser button opens your wishlist. With a private profile it says so instead of failing.
- [ ] Store > Browse: Top Sellers, New Releases, Specials and Coming Soon switch the list; genre chips and "More tags" narrow it; scrolling to the bottom loads more cards; a click opens the page.
- [ ] Prices show your currency (check against the store in a browser); with a different language in Settings > Downloads the store texts follow (cached answers are per language).

## 8. Sign out

- [ ] Settings → Privacy → Sign out: you're back at the QR screen.
- [ ] The keyring entry is gone and the account's library cache is deleted.
- [ ] Known limit: the session stays listed under authorized devices in the Steam Mobile App until you remove it there.

## Menu bar, history, tray and animations

- [ ] Click "View": its menu opens. With it open, move over "Friends", "Games", "Help": each opens at once without a click. Click outside, or press Esc: it closes. With a menu open, Left/Right switch menus. F10 opens the first menu.
- [ ] Kettle menu: Settings opens the settings window; Check for Kettle updates opens the GitHub releases page; Exit really quits (also with the tray on).
- [ ] View > Zoom in / out / reset, and Ctrl + / Ctrl - / Ctrl 0, change the interface scale; it is kept after a restart (Settings > Interface).
- [ ] Help > System information shows OS, kernel, desktop, session, graphics adapter, Steam folders, Proton count; Copy puts it on the clipboard. About shows version, license and "not affiliated with Valve".
- [ ] Open the Store, a store page, Library, a game, Downloads. The title bar back and forward buttons, mouse button 4 and 5 (side buttons), and Alt+Left / Alt+Right walk through them in order. Esc goes back (not while a menu or dialog is open or you are typing).
- [ ] On a game's page Enter or Space plays an installed game.
- [ ] Settings > Interface > Animations: Off makes page changes, hovers and the tab underline instant. Reduced fades but nothing slides. Idle CPU stays near 0 in all three.
- [ ] Tray (Settings > Interface): the icon shows in the KDE tray with Kettle, Store, Library, Friends, Downloads, recent games (Play), Settings, Exit. Left click shows and raises the window. With "Closing the window keeps Kettle in the tray" on, the close button hides the window (on Wayland it minimizes it) and the tray brings it back. Exit really quits. Without a tray host the close button quits as before.

## Windows

- [ ] Unzip `dist/kettle-windows-x86_64.zip` and run `kettle.exe`. It should find Steam games installed in `C:\Program Files (x86)\Steam` and in other libraries.
- [ ] Sign in, install a small game and play it.

Write down anything that looks off, along with a screenshot.

## Friends and profiles (real account)
- [ ] Sign in. The Friends panel header shows your profile name (not the account name) and "Online". Picking another state sticks after a restart; the default for a new install is Online.
- [ ] The list shows only your real friends with names and avatars (nobody shown as a number; a friend whose name hasn't arrived yet shows "Loading..." in grey). IN-GAME friends show "In-Game: <game>", offline ones "Last online 6 min ago". Sections show counts and collapse.
- [ ] Compare with the official client's friends window: same friends, same sections and order (offline: most recently online first).
- [ ] Someone sends you a friend request: FRIEND REQUESTS appears with Accept / Ignore; Accept moves them into the list. A request you sent shows under PENDING with Cancel.
- [ ] "+" next to the search box: enter a friend code, a SteamID64 and a `steamcommunity.com/profiles/...` link in turn: each sends a request (a vanity `/id/name` link says it needs a code or profiles link).
- [ ] Right-click a friend: Send message, View profile, Remove friend and Block each ask for confirmation where it matters; Remove friend removes them, Block hides them.
- [ ] GROUP CHATS lists your chat groups. Click one: its default channel opens, history loads, a message you send appears for the others, theirs appear here. The channel picker under the group name switches channels.
- [ ] Click your name or avatar in the panel header, or the profile tab: your profile page shows avatar, name, level, real name and location (if set), summary and recent games. Edit profile: change the summary, Save; the page shows the new text. Change the profile name: the panel header shows it a moment later.
- [ ] View profile on a friend and on a private profile ("This profile is private."). No token or cookie appears in the terminal output or in any error message.


## Steam Cloud saves (real account; written, not yet tried)
Use a small game that has Steam Cloud and starts under Kettle. Open the launch log (`~/.local/share/kettle/logs/launch-<appid>.log`) after each step: it starts with `cloud:` lines. Set `KETTLE_CLOUD_DRYRUN=1` first to see the plan without any change.
- [ ] Dry run: start Kettle with `KETTLE_CLOUD_DRYRUN=1`, play the game. The log lists `download` / `upload` lines and `dry run, nothing changed`; no save file or `~/.local/share/kettle/cloud/<appid>.json` changed.
- [ ] Download: save progress on another machine (or the official client), close the game there, then start the game in Kettle. "Syncing..." shows, the log says `downloaded <file>`, and the game loads that save.
- [ ] Upload: play, save, quit. The log ends with `uploaded <file>`; in the official client (or on another machine) the new save loads.
- [ ] Conflict: change saves here without Kettle noticing (offline), change them elsewhere, go online and press Play. The dialog shows both times and the game does not start. "Cancel" leaves everything as it was. "Keep cloud saves" replaces the local ones (the old ones are in `~/.local/share/kettle/cloud-backups/<appid>/`). "Keep this computer's saves" uploads them first. Both then start the game.
- [ ] Proton game: saves land inside `compatdata/<appid>/pfx/drive_c/users/steamuser/...` (check the path in the log). A game whose prefix does not exist yet says so and starts without Cloud; the next launch syncs.
- [ ] Per-game off and the global switch: with either off the log has no `cloud:` lines and nothing is sent. Settings survive a restart.
- [ ] Failure: disconnect the network just before Play (or sign out of the hub): the game still starts, a notice says Cloud could not sync, and nothing is uploaded after the session.
- [ ] No save file is ever deleted by Kettle; a file deleted in the cloud stays on disk.
- [ ] With the official Steam client installed: a game that keeps its saves in `userdata/<account>/<app>/remote` is not synced for that folder; the launch log says `Cloud: remote/ belongs to the Steam client, skipped` and the game page shows it. Nothing under the Steam folder changes (check modification times). Saves in the game folder, home or the Proton prefix still sync.

## Alpha 6: quality-of-life features (owner steps)

These are unit-tested but not tried on the desktop yet.

### Kettle updates itself (run the AppImage, not `cargo run`)
- [ ] Settings shows "Check for Kettle updates" on. With a newer release on GitHub (a pre-release counts), a notice "Update available: Kettle X" appears once and the banner / settings row offers "Update now" and "Later".
- [ ] "Update now" shows the download progress, then "Restart now". Restart: the window comes back as the new version (Settings shows its version). The old file was replaced in place (`ls -l` the AppImage); no `.update.tmp` file is left next to it.
- [ ] Turn the setting off and restart: no request is made. With no network, or a private repository, nothing is shown (no error).
- [ ] Run Kettle from `cargo run`: "Check now" says updating itself needs the AppImage version.

### Start with the computer
- [ ] Turn it on: `~/.config/autostart/kettle.desktop` exists and its `Exec=` is the AppImage path. Turn on "Start minimized": the line ends with `--minimized`. Log out and in: Kettle starts hidden in the tray.
- [ ] Turn it off: the file is gone. Restart Kettle with the file present: the switch shows on.

### Friend notifications
- [ ] Settings > Notifications: "A friend joins a game" is on, "A friend comes online" is off. With the Kettle window unfocused, a friend starting a game shows a notification; a friend coming online shows one only after you turn that on.
- [ ] Right after sign-in (first 30 seconds) no notifications appear; the same friend does not notify again within 5 minutes; you never get one about yourself.

### ProtonDB rating
- [ ] Open a Windows-only game's page: "ProtonDB: Gold" (or the right tier) appears; hovering shows the number of reports; clicking opens protondb.com. A game with a native Linux build shows nothing. `~/.cache/kettle/protondb/<id>.json` exists; opening the game again within a day makes no request.

### Per-game environment variables
- [ ] Game properties > Compatibility: turn on "Show the DXVK frame rate", type `FOO=bar`, Apply. Start the game: the launch log (`~/.local/share/kettle/logs/launch-<id>.log`) lists `env: DXVK_HUD=fps` and `env: FOO=bar`. A bad line (`1X=y`) shows an error and is not saved. With GameMode installed, "Use GameMode" makes the command start with `gamemoderun`.

### Non-Steam games
- [ ] Add a Game: type the full path of a Linux program (or pick an installed app: name, program and icon fill in). It appears in the library; Play starts it; the play time grows after it exits.
- [ ] Add a `.exe`: it starts through Proton (default tool) with a prefix in `~/.local/share/kettle/compatdata/<id>`.
- [ ] "Import from Steam" adds the shortcuts of the official client; Steam's `shortcuts.vdf` is unchanged (compare its modification time). "Remove from library" takes a game off the list and leaves its files.
