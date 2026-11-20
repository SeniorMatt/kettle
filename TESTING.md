# Manual test checklist

Things the automated checks can't cover: a real desktop, a real account and real games. Run them on a real machine (Linux first), top to bottom. Each step says what to do and what you should see.

## 0. Get the build

- AppImage: copy `dist/steam-rust-x86_64.AppImage` to the machine, then `chmod +x steam-rust-x86_64.AppImage && ./steam-rust-x86_64.AppImage`.
- Or from source: `cargo run --release -p steam-rust`.
- Timings: start it with `STEAM_RUST_TIMINGS=1` and note the `[timing]` lines printed in the terminal.

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
- [ ] Look in the keyring (KWallet Manager or Seahorse): there's one `steam-rust` entry. Check `~/.local/share/steam-rust/` too: no token in any file.

## 3. Library

- [ ] Games installed by the official client show as installed, with their size.
- [ ] Scrolling the grid stays smooth. Hover lifts a cover a little.
- [ ] Sidebar search filters as you type.
- [ ] Sorting (Alphabetical / Last Played / Hours Played) works.
- [ ] With the window idle, CPU is about 0% in `top`.

## 4. Downloads

Pick a small game you own (under 2 GB).

- [ ] On its page, INSTALL → the dialog shows the libraries (ours and Steam's) and the free space.
- [ ] Install into the steam-rust library. The Downloads page and the bottom bar show the speed and percentage.
- [ ] Pause, wait, then Resume: it continues from where it stopped.
- [ ] When it finishes, the game page shows PLAY and the size on disk.
- [ ] Gear → Verify files: it finishes with nothing re-downloaded.
- [ ] Gear → Browse local files: the game's folder opens.

## 5. Play

- [ ] A native Linux game: PLAY → it starts and the button shows RUNNING. Quit the game: play time goes up.
- [ ] A Windows game already installed by Steam: PLAY → it starts through Proton, with the same saves as in Steam.
- [ ] Gear → Compatibility…: pick another Proton; the game uses it the next time.
- [ ] If a game won't start, check the log path in the message (`~/.local/share/steam-rust/logs/`). Then try Gear → Launch via Steam client.
- [ ] Known limit: games that need the Steam client running (most Steamworks games) only start while the official Steam client is running.

## 6. Proton manager

- [ ] Settings → Compatibility lists Steam's Protons and any custom ones.
- [ ] Install the latest GE-Proton: the progress bar moves and it then shows as installed.
- [ ] Use it for one game, play, and then Remove it.

## 7. Store

- [ ] Store tab: the carousel, specials and search all work.
- [ ] Open a game, then step through its screenshots. Add to cart opens your browser.

## 8. Sign out

- [ ] Settings → Privacy → Sign out and revoke: you're back at the QR screen.
- [ ] The keyring entry is gone and the account's library cache is deleted.
- [ ] In the Steam Mobile App, under authorized devices, the steam-rust session is gone.

## Windows

- [ ] Unzip `dist/steam-rust-windows-x86_64.zip` and run `steam-rust.exe`. It should find Steam games installed in `C:\Program Files (x86)\Steam` and in other libraries.
- [ ] Sign in, install a small game and play it.

Write down anything that looks off, along with a screenshot.
