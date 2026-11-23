# Kettle roadmap

Everything the official Steam client does, as one checklist. `- [x]` means it is built and has been tested at least once (in the app, the command-line tool or unit tests). `- [ ]` means it isn't built yet, or doesn't work on a real account yet. The area comes before the first colon. `python3 scripts/status.py` counts the lines and draws the picture in the README.

Nothing here is a promise. The owner decides what comes next. Bugs found on real machines go first.

## Account

- [x] Account: QR sign-in with the Steam Mobile App
- [x] Account: refresh token kept in the system keyring, never in a file
- [x] Account: sign out deletes the token and the cached library
- [ ] Account: sign out revokes the token on Steam's side
- [x] Account: remembered accounts and instant start from cache
- [ ] Account: sign in with a password and Steam Guard code
- [ ] Account: switch between several accounts
- [ ] Account: Steam Guard confirmations (trades, market)
- [ ] Account: account details page (email, security, licences)
- [ ] Account: parental controls / Family View PIN

## Library

- [x] Library: grid of covers that stays smooth with thousands of games
- [x] Library: sidebar list with search
- [x] Library: sort by name, last played, hours played
- [x] Library: finds games the native Steam installed
- [x] Library: installed games show even without the network or the owned list
- [ ] Library: finds games Flatpak Steam installed (re-test on the T14s)
- [x] Library: owned games load on a real account (SteamClient token over CM)
- [x] Library: collections (yours and the official client's, read-only)
- [ ] Library: dynamic collections
- [x] Library: filters (installed, ready to play)
- [ ] Library: filters (genre, players, features)
- [x] Library: favourites and hidden games
- [ ] Library: home shelves (recent games, what's new)
- [ ] Library: list view with columns
- [ ] Library: custom artwork per game
- [x] Library: non-Steam games (Add a Game, import of Steam's shortcuts) (unit-tested; not tried on the desktop yet)
- [ ] Library: Family Sharing libraries
- [x] Library: game page with hero art, play time, last played
- [x] Library: game page activity feed and news (parser and layout tested, demo shots; not tried on a real account)
- [x] Library: game page achievements summary (KeyValues reader tested with own fixtures; not tried on a real account)
- [x] Library: game page friends who play (in-game friends only; "friends who own it" is not known to Kettle; not tried on a real account)
- [x] Library: game page DLC list (names from PICS over the signed-in connection; installed mark only, no owned mark; not tried on a real account)
- [x] Library: game properties window (launch options, compatibility, installed files)
- [ ] Library: game properties: language and betas

## Store

- [x] Store: featured carousel, specials, new and trending, coming soon
- [x] Store: search
- [x] Store: game pages with screenshots and price
- [x] Store: videos and trailers (unit-tested parsing and link allowlist; opens in the system player, not yet tried by hand)
- [x] Store: reviews (unit-tested parsing, not yet tried by hand)
- [x] Store: wishlist (read-only, unit-tested parsing, not yet tried by hand; adding and removing stays on the store page)
- [ ] Store: cart and checkout inside the app
- [x] Store: browse by tag, genre and category (unit-tested parsing, not yet tried by hand)
- [ ] Store: discovery queue
- [x] Store: your region's currency and language (country from the login, language from Settings; unit-tested, not yet tried by hand)
- [ ] Store: sales events and points shop

## Downloads

- [x] Downloads: download a game from Steam's content servers
- [x] Downloads: queue with pause, resume and cancel
- [x] Downloads: verify and repair files
- [x] Downloads: uninstall
- [x] Downloads: Steam-format libraries the official client can read
- [x] Downloads: Windows depots for Proton games
- [ ] Downloads: download with a real account (owned paid games)
- [x] Downloads: automatic game updates (check at start and every 45 min, per-game and schedule settings, a dialog before touching the official client's folders; unit-tested, not yet tried on a real update)
- [x] Downloads: delta patches (only changed chunks of updates)
- [x] Downloads: bandwidth limit and schedule (the schedule covers automatic updates only)
- [ ] Downloads: beta branches and beta passwords
- [ ] Downloads: choose DLC to install (install dialog built, not tested yet)
- [ ] Downloads: language choice for new installs (built, not tested yet)
- [x] Downloads: desktop and application menu shortcuts
- [ ] Downloads: move a game to another library (built, not tested yet)
- [ ] Downloads: storage manager (sizes per library, clean up) (built, not tested yet)
- [ ] Downloads: shader pre-caching
- [ ] Downloads: Workshop items

## Play

- [x] Play: start native Linux games ourselves
- [x] Play: start Windows games through Proton with Steam's prefixes (shared saves)
- [x] Play: umu-launcher support
- [x] Play: running state, stop, play-time tracking
- [x] Play: start the official client in the background for Steamworks games
- [x] Play: Steam is warm before Play (Automatic/Always/Never, low priority, per-install scan cache, IPC readiness instead of a fixed wait)
- [ ] Play: Steam warm start timed on the owner's machine (start to IPC ready)
- [ ] Play: tested on a real game on a real machine
- [x] Play: launch options and command-line arguments per game
- [x] Play: run games in Gamescope (option; unit-tested, not tried in a session yet)
- [ ] Play: pre-launch checks (first-time setup, redistributables)
- [ ] Play: play time synced with Steam's servers
- [x] Play: own steamclient core (exports, ISteamClient/User/Apps/Utils/UserStats/Friends tables, null objects, socket host; see docs/steamclient-design.md; built, tested by loading it like a game against a fake host)
- [x] Play: own steamclient vtables compared with the public headers (Client 017-023, User 017-023, Friends 015-018, Utils 007-011, Apps 006-009, UserStats 011-013; Input/RemoteStorage/NetworkingUtils and the rest as do-nothing objects)
- [x] Play: setting "Run games without the Steam client (experimental)" (off by default; Kettle-owned Steam root, host wired to the session)
- [ ] Play: own steamclient run by a real Steamworks game on Linux (dlopen test with Valve's libsteam_api.so written; needs a machine with one)
- [ ] Play: own steamclient tickets (ownership and encrypted app tickets over the CM, built; auth session ticket framing and DRM-stub interface not yet)
- [ ] Play: own steamclient stats and achievements synced with Steam (local file only so far; needs the stats schema)
- [ ] Play: own steamclient 32-bit build (i686) for 32-bit games
- [ ] Play: own steamclient Proton bridge install and Kettle-owned Steam root (only when the official client is not installed)
- [ ] Play: own steamclient Cloud saves, rich presence, leaderboards
- [ ] Play: own steamclient networking, matchmaking, lobbies
- [ ] Play: own steamclient Steam Input, Workshop/UGC, overlay, voice
- [ ] Play: Remote Play
- [ ] Play: Steam Input (controller configuration)

## Compatibility

- [x] Compatibility: list Steam's Protons and custom tools
- [x] Compatibility: install GE-Proton, Proton-CachyOS, Proton-EM, Proton-Sarek, Luxtorpeda
- [x] Compatibility: per-game choice, using Steam's choice by default
- [x] Compatibility: SteamLinuxRuntime container
- [ ] Compatibility: update notifications for installed tools
- [x] Compatibility: per-game environment variables (DXVK, MangoHud, gamemode) (unit-tested; not tried on the desktop yet)
- [x] Compatibility: ProtonDB rating on game pages (unit-tested; not tried on the desktop yet)

## Cloud

- [x] Cloud: download saves before playing (unit-tested; not tried on a real account)
- [x] Cloud: upload saves after playing (unit-tested; not tried on a real account)
- [x] Cloud: conflict dialog (decision and flow unit-tested; the dialog is in `cloud_ui.rs` and is not hooked into the pages yet; not tried on a real account)
- [ ] Cloud: Steam Cloud for non-Steam-Cloud games (local backups)

## Friends

- [x] Friends: friends list with states, groups and avatars (demo)
- [x] Friends: set your own state (online, away, invisible, offline) (demo)
- [ ] Friends: works on a real account
- [ ] Friends: friend requests (accept, ignore, cancel) and add a friend by code or link (built, not yet tried on a real account)
- [ ] Friends: blocking, and a list of blocked accounts to unblock
- [ ] Friends: friend profiles pop-up (a full profile page is coded; the small pop-up is not)
- [ ] Friends: invite to game and join game
- [ ] Friends: nicknames and favourites
- [x] Friends: notifications for friends coming online (and for starting a game) (unit-tested; not tried on the desktop yet)

## Chat

- [x] Chat: one-to-one chat with history and unread counts (demo)
- [ ] Chat: works on a real account
- [ ] Chat: group chats: list, read and send in a channel (built, not yet tried on a real account)
- [ ] Chat: group chats: create, join, invite, roles, voice
- [ ] Chat: voice chat
- [ ] Chat: images, links and embeds
- [ ] Chat: emoticons and stickers
- [ ] Chat: typing indicator

## Overlay

- [x] Overlay: Shift+Tab while our game runs (X11)
- [x] Overlay: friends, chat, notes, session time
- [ ] Overlay: tested on KDE Wayland
- [x] Overlay: drawn inside the game's frames (OpenGL preload, Vulkan layer), pinned to the game; takes the game's input while open (SDL2, SDL3, Xlib)
- [ ] Overlay: tested with real games (DDNet, a Proton game); 32-bit games; Wayland-native input
- [ ] Overlay: draws on top of exclusive-fullscreen games (Vulkan layer; built, untested)
- [ ] Overlay: inside Gamescope (the library is not loaded there; the in-process window is the fallback)
- [ ] Overlay: web browser
- [ ] Overlay: achievements and guides
- [x] Overlay: FPS counter (drawn by the overlay library; MangoHud inside Gamescope)

## Community

- [ ] Community: your profile: view and edit name, real name and summary (built, not yet tried on a real account)
- [ ] Community: your profile: avatar upload, location, privacy settings, showcases
- [ ] Community: other people's profiles: public page with summary, level and games (built, not yet tried)
- [ ] Community: other people's profiles: badges, friends count, comments
- [ ] Community: achievements per game
- [ ] Community: screenshots (take with F12, manage, upload)
- [ ] Community: game recording and clips
- [ ] Community: Workshop browser and subscriptions
- [ ] Community: community hubs and discussions
- [ ] Community: guides
- [ ] Community: inventory
- [ ] Community: trading
- [ ] Community: market
- [ ] Community: badges and trading cards
- [ ] Community: broadcasting

## Notifications

- [x] Notifications: download finished, in-app message for new chat messages
- [ ] Notifications: desktop notifications (settings and download-finished built, not tested yet; friend and chat events wait for the friends code to call it)
- [ ] Notifications: gifts, comments, trade offers

## Settings

- [x] Settings: downloads (default library, parallel downloads)
- [x] Settings: compatibility page
- [x] Settings: privacy page with sign-out
- [x] Settings: games page (overlay, Steam client helper)
- [x] Settings: looks like the official client's settings window
- [ ] Settings: interface: start page, scale, animations (built, not tested yet)
- [ ] Settings: interface: language
- [ ] Settings: library folders management (add and remove our folders; built, not tested yet)
- [ ] Settings: in-game (screenshot key, overlay key; performance overlay is built, not yet tested)
- [ ] Settings: controller
- [ ] Settings: minimize to tray, start minimized (built, not tested yet)
- [x] Settings: start with the computer (unit-tested; not tried on the desktop yet)

## Interface

- [x] Interface: custom title bar, resize edges, Alt+drag, Ctrl+Tab
- [x] Interface: subtle animations, near-zero idle CPU
- [x] Interface: closing the window is immediate (0.1-0.4 s, X11)
- [x] Interface: menus and drop-downs look like Steam
- [ ] Interface: system tray icon (built, not tested yet; Linux only)
- [x] Interface: menu bar like the official client's, mouse 4/5 history
- [ ] Interface: Big Picture / gamepad UI
- [ ] Interface: translations (Russian first)
- [ ] Interface: keyboard navigation: Esc back, Enter/Space plays, Alt+Left/Right (built, not tested yet)
- [ ] Interface: keyboard navigation: focus ring in the library grid and sidebar, Ctrl+F
- [ ] Interface: keyboard navigation everywhere else

## Platform

- [x] Platform: Linux AppImage
- [x] Platform: Windows build
- [x] Platform: privacy (no telemetry, tokens only in the keyring)
- [ ] Platform: tested on real Windows
- [ ] Platform: Flatpak package
- [ ] Platform: .deb / AUR packages
- [x] Platform: auto-update of Kettle itself (AppImage) (unit-tested; not tried on the desktop yet)
- [ ] Platform: macOS
