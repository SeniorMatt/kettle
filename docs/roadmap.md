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
- [ ] Library: non-Steam games (Add a Game)
- [ ] Library: Family Sharing libraries
- [x] Library: game page with hero art, play time, last played
- [ ] Library: game page activity feed and news
- [ ] Library: game page achievements summary
- [ ] Library: game page friends who play
- [ ] Library: game page DLC list
- [x] Library: game properties window (launch options, compatibility, installed files)
- [ ] Library: game properties: language and betas

## Store

- [x] Store: featured carousel, specials, new and trending, coming soon
- [x] Store: search
- [x] Store: game pages with screenshots and price
- [ ] Store: videos and trailers
- [ ] Store: reviews
- [ ] Store: wishlist
- [ ] Store: cart and checkout inside the app
- [ ] Store: browse by tag, genre and category
- [ ] Store: discovery queue
- [ ] Store: your region's currency and language
- [ ] Store: sales events and points shop

## Downloads

- [x] Downloads: download a game from Steam's content servers
- [x] Downloads: queue with pause, resume and cancel
- [x] Downloads: verify and repair files
- [x] Downloads: uninstall
- [x] Downloads: Steam-format libraries the official client can read
- [x] Downloads: Windows depots for Proton games
- [ ] Downloads: download with a real account (owned paid games)
- [ ] Downloads: automatic game updates
- [ ] Downloads: delta patches (only changed chunks of updates)
- [ ] Downloads: bandwidth limit and schedule
- [ ] Downloads: beta branches and beta passwords
- [ ] Downloads: choose DLC to install
- [ ] Downloads: move a game to another library
- [ ] Downloads: storage manager (sizes per library, clean up)
- [ ] Downloads: shader pre-caching
- [ ] Downloads: Workshop items

## Play

- [x] Play: start native Linux games ourselves
- [x] Play: start Windows games through Proton with Steam's prefixes (shared saves)
- [x] Play: umu-launcher support
- [x] Play: running state, stop, play-time tracking
- [x] Play: start the official client in the background for Steamworks games
- [ ] Play: tested on a real game on a real machine
- [x] Play: launch options and command-line arguments per game
- [ ] Play: pre-launch checks (first-time setup, redistributables)
- [ ] Play: play time synced with Steam's servers
- [ ] Play: games run without the official client (own Steamworks service)
- [ ] Play: Remote Play
- [ ] Play: Steam Input (controller configuration)

## Compatibility

- [x] Compatibility: list Steam's Protons and custom tools
- [x] Compatibility: install GE-Proton, Proton-CachyOS, Proton-EM, Proton-Sarek, Luxtorpeda
- [x] Compatibility: per-game choice, using Steam's choice by default
- [x] Compatibility: SteamLinuxRuntime container
- [ ] Compatibility: update notifications for installed tools
- [ ] Compatibility: per-game environment variables (DXVK, MangoHud, gamemode)
- [ ] Compatibility: ProtonDB rating on game pages

## Cloud

- [ ] Cloud: download saves before playing
- [ ] Cloud: upload saves after playing
- [ ] Cloud: conflict dialog
- [ ] Cloud: Steam Cloud for non-Steam-Cloud games (local backups)

## Friends

- [x] Friends: friends list with states, groups and avatars (demo)
- [x] Friends: set your own state (online, away, invisible, offline) (demo)
- [ ] Friends: works on a real account
- [ ] Friends: friend requests and blocking
- [ ] Friends: friend profiles pop-up
- [ ] Friends: invite to game and join game
- [ ] Friends: nicknames and favourites
- [ ] Friends: notifications for friends coming online

## Chat

- [x] Chat: one-to-one chat with history and unread counts (demo)
- [ ] Chat: works on a real account
- [ ] Chat: group chats and channels
- [ ] Chat: voice chat
- [ ] Chat: images, links and embeds
- [ ] Chat: emoticons and stickers
- [ ] Chat: typing indicator

## Overlay

- [x] Overlay: Shift+Tab while our game runs (X11)
- [x] Overlay: friends, chat, notes, session time
- [ ] Overlay: tested on KDE Wayland
- [ ] Overlay: draws on top of exclusive-fullscreen games (Vulkan layer)
- [ ] Overlay: web browser
- [ ] Overlay: achievements and guides
- [ ] Overlay: FPS counter

## Community

- [ ] Community: your profile (view and edit)
- [ ] Community: other people's profiles
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
- [ ] Notifications: desktop notifications
- [ ] Notifications: gifts, comments, trade offers

## Settings

- [x] Settings: downloads (default library, parallel downloads)
- [x] Settings: compatibility page
- [x] Settings: privacy page with sign-out
- [x] Settings: games page (overlay, Steam client helper)
- [x] Settings: looks like the official client's settings window
- [ ] Settings: interface (language, start page, scale)
- [ ] Settings: library folders management
- [ ] Settings: in-game (screenshot key, overlay key)
- [ ] Settings: controller
- [ ] Settings: start with the computer, minimize to tray

## Interface

- [x] Interface: custom title bar, resize edges, Alt+drag, Ctrl+Tab
- [x] Interface: subtle animations, near-zero idle CPU
- [x] Interface: closing the window is immediate (0.1-0.4 s, X11)
- [x] Interface: menus and drop-downs look like Steam
- [ ] Interface: system tray icon
- [ ] Interface: Big Picture / gamepad UI
- [ ] Interface: translations (Russian first)
- [ ] Interface: keyboard navigation everywhere

## Platform

- [x] Platform: Linux AppImage
- [x] Platform: Windows build
- [x] Platform: privacy (no telemetry, tokens only in the keyring)
- [ ] Platform: tested on real Windows
- [ ] Platform: Flatpak package
- [ ] Platform: .deb / AUR packages
- [ ] Platform: auto-update of Kettle itself
- [ ] Platform: macOS
