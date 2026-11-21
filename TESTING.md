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

## 4. Downloads

Pick a small game you own (under 2 GB).

- [ ] On its page, INSTALL → the dialog shows the libraries (ours and Steam's) and the free space.
- [ ] Install into the kettle library. The Downloads page and the bottom bar show the speed and percentage.
- [ ] Pause, wait, then Resume: it continues from where it stopped.
- [ ] When it finishes, the game page shows PLAY and the size on disk.
- [ ] Gear → Verify files: it finishes with nothing re-downloaded.
- [ ] Gear → Browse local files: the game's folder opens.

## 5. Play

- [ ] A native Linux game: PLAY → it starts and the button shows RUNNING. Quit the game: play time goes up.
- [ ] A Windows game already installed by Steam: PLAY → it starts through Proton, with the same saves as in Steam.
- [ ] Gear → Compatibility…: pick another Proton; the game uses it the next time.
- [ ] If a game won't start, check the log path in the message (`~/.local/share/kettle/logs/`). Then try Gear → Launch via Steam client.
- [ ] Steamworks games: with Settings → Games → "Start the Steam client in the background" on, PLAY on a game that ships `libsteam_api.so` / `steam_api64.dll` shows STARTING STEAM… and then starts the official client minimized (`-silent`) and the game. With it off, or without an installed client, the game starts anyway; if it exits within 10 s a notice says it needs the Steam client. `sr-cli launch <appid> --dry-run` prints "needs the Steam client: yes/no, running: yes/no" and starts nothing.

### In-game overlay (Shift+Tab)

- [ ] KDE Wayland: start a game from the launcher. KDE shows a one-time "kettle wants to register a shortcut" prompt; accept it. System Settings → Shortcuts lists kettle "Show or hide the in-game overlay" with Shift+Tab.
- [ ] With the game in focus (also fullscreen), Shift+Tab shows the dimmed overlay over it with a fade-in: game name, session time, clock, friends and chat, Notes, "Return to game".
- [ ] Shift+Tab, Esc or "Return to game" hides it and the game has focus again (keyboard and mouse work).
- [ ] Type a note, close, press Shift+Tab again: the note is still there. Restart the app: it is still there.
- [ ] With no game running, Shift+Tab does nothing here: other programs get it (e.g. Shift+Tab moves focus backwards in a browser). Quit the game: same.
- [ ] Settings → Games → "Enable the in-game overlay" off: Shift+Tab is no longer taken and Games → "Show overlay" is gone.
- [ ] X11 session: the same checks (the shortcut is a key grab, no prompt).
- [ ] Windows: same checks with a fullscreen-windowed game. If the overlay shows behind an exclusive-fullscreen game, note it.
- [ ] While the overlay is hidden the app uses no CPU (`top`).

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

## 8. Sign out

- [ ] Settings → Privacy → Sign out: you're back at the QR screen.
- [ ] The keyring entry is gone and the account's library cache is deleted.
- [ ] Known limit: the session stays listed under authorized devices in the Steam Mobile App until you remove it there.

## Windows

- [ ] Unzip `dist/kettle-windows-x86_64.zip` and run `kettle.exe`. It should find Steam games installed in `C:\Program Files (x86)\Steam` and in other libraries.
- [ ] Sign in, install a small game and play it.

Write down anything that looks off, along with a screenshot.
