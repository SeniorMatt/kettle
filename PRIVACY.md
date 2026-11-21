# Kettle

Kettle has no telemetry, no analytics, no crash reporting and no servers of its own. It only talks to Valve's servers, the same ones the official client uses.

## Signing in

- **QR code only.** You approve the sign-in in the Steam Mobile App, so a password is never typed into, seen by, or stored by this app.
- **The refresh token** (it keeps you signed in) is stored only in your system keyring: GNOME Keyring, KWallet or KeePassXC, through the Secret Service API. It's never written to a file, a log, the screen or `Debug` output.
- **Access tokens** are short-lived. They're requested when needed and kept in memory only.
- When a session ends, tokens are overwritten in memory.
- **Sign out** deletes the token from the keyring and the cached library of that account. It does not yet revoke the token on Steam's servers. To end the session there too, remove "Kettle" under authorized devices in the Steam Mobile App (or "Deauthorize all other devices" on the Steam website).
- **Error messages are scrubbed:** anything that looks like a token is replaced with `[redacted]` before it's shown.
- The device name sent to Steam is just `Kettle (Linux)` (or `Kettle (Windows)`). It doesn't include your hostname or username.

## Files we write

All files are readable only by you: folders are mode `0700` and files `0600`.

| Path | Contents |
|---|---|
| `~/.local/share/kettle/accounts.json` | Account names and SteamIDs you've signed in with (no secrets) |
| `~/.cache/kettle/library-<steamid>.json` | Your owned-games list, for instant startup |
| `~/.local/share/kettle/settings.json` | Your default download folder and the number of parallel downloads |
| `~/.local/share/kettle/playtime.json` | Play time we measured ourselves, per game (never sent anywhere) |
| `~/.local/share/kettle/collections.json` | Your favourites, hidden games and collections made in Kettle |
| `~/.local/share/kettle/compat_choices.json` | Which compatibility tool you picked per game |
| `~/.local/share/kettle/launch_options.json` | The launch options you typed per game (never sent anywhere) |
| `~/.local/share/kettle/gamescope_games.json` | Which games always or never run in Gamescope (the global switch is in `settings.json`) |
| `$XDG_RUNTIME_DIR/kettle-gamescope-<app id>.display` | While a game runs in Gamescope: the name of its private X display (`:1`), written by our launch shim, removed at the next launch |
| `~/.local/share/kettle/logs/` | Output of the last run of each game we started |
| `~/.local/share/kettle/compatibilitytools.d/` | Compatibility tools (Proton, ...) you installed from the app |
| `~/.local/share/kettle/library/` | Games installed by the app, unless you pick another folder |
| `~/.cache/kettle/art/` | Cover art downloaded from Steam's CDN |
| `~/.cache/kettle/store/` | Public store answers and images, kept briefly to make the Store tab fast |

If a game needs the Steam client (Settings, Games tab, on by default), we start the official client minimized with `-silent` before it. We only start the program and read its pid file; nothing is sent to it or to Valve by us for this.

We **never write** to the official Steam client's files. We only read its `steamapps/*.acf`, `libraryfolders.vdf`, `appcache/librarycache` and, for your collections, favourites and hidden games, `userdata/<account>/config/cloudstorage/cloud-storage-namespace-1.json` (read only; ours are kept separately in `collections.json`).

## Network connections

| Host | Why |
|---|---|
| `api.steampowered.com` | QR sign-in and the CM server list. Also the public store lookup `IStoreBrowseService/GetItems` (no login; only the app IDs, the country and language) to find cover art and icons that live at hashed paths, and names and prices for the wishlist and Browse lists. `IWishlistService/GetWishlist` (your own public SteamID; no login, so it only works when your profile's game details are public) and `IStoreService/GetTagList` (the genre and tag names) |
| Steam CM servers (`*.steamserver.net`) | The client connection (encrypted): your owned games, downloads and short-lived access tokens. After sign-in it also carries Friends & Chat: your friends list, their names and presence, your own online state (Online by default; you can pick Away, Busy, Invisible or Offline in the panel) and chat messages |
| `shared.steamstatic.com` | Cover art (only the app ID is sent) |
| `media.steampowered.com` | Small game icons |
| `store.steampowered.com` | Public store data for the Store tab: featured lists, search and app details. No login or cookies; only the country and language are sent (`cc=` and `l=`), plus the search text, app ID, tag ID and list offset you look at. The country is the one Steam reported when you signed in (from your IP address); before that, or if it is missing, no country is sent and Steam picks one from your IP address. The language is the one in Settings > Downloads (English by default). Store pages also request the app's user reviews (`/appreviews/<app id>`) and Browse uses `/search/results/?json=1`. Kettle never adds or removes wishlist entries, buys anything or sends cookies |
| `*.steamstatic.com` | Store screenshots and capsules (only the image URL is requested). Trailers are not played in the app: a click hands the video address (on `video.*.steamstatic.com`) to your system's video player or browser |
| `avatars.steamstatic.com` | Friends' profile pictures for the Friends & Chat panel (only the picture's hash is in the URL; kept in the same image cache) |

All HTTPS uses rustls with certificate verification.

## Friends and chat

Chat messages and your friends list are held in memory only (at most 2000 friends and 500 messages per chat) and are never written to disk. Messages go to Steam over the CM connection, the same way the official client sends them. Only avatar pictures are cached on disk (see above). The one thing remembered between runs is the online state you picked (`friends.json` in our data folder).

## Profiles

Opening someone's profile asks the community website for two public pages, without any login: `https://steamcommunity.com/profiles/<SteamID>?xml=1` (name, summary, location, privacy) and `https://steamcommunity.com/miniprofile/<account id>/json` (Steam level). The answers are kept in memory only (at most 24 profiles). The picture comes from `avatars.steamstatic.com` like the friends' pictures.

Editing your own profile (name, real name, summary) uses the community website's own form. For each load or save we ask Steam for a short-lived access token over the CM connection and send it to `steamcommunity.com` as the cookie `steamLoginSecure`, together with a random `sessionid` cookie and form field, the same way the website does. The cookies exist only in memory for the length of that one request: they are never written to disk or the log, and error messages have them removed before they are shown.

## Launch options

When you have not set launch options for a game, we show and use the ones the official Steam client saved for it. They are read, never written, from `userdata/<account id>/config/localconfig.vdf` in your Steam folder, and only for the signed-in account.

## In-game overlay

The Shift+Tab shortcut is registered only while a game started here is running (and the setting is on); nothing is sent anywhere. Notes are saved per game in `notes.json` in our data folder.

The overlay is drawn by a second process, `kettle --overlay`, started from the same program file while such a game runs and ended with it. It talks to the main app over a Unix socket in your runtime folder (`$XDG_RUNTIME_DIR/kettle-overlay-<pid>.sock`, readable only by you, removed when the game ends). Nothing leaves your computer through it. What goes over it: your friends' names and states, the open chats, the game's note and the performance overlay settings; and back: chat messages you type, note text and settings you change. The helper has no Steam sign-in or token.

## Gamescope (optional)

If you turn on "Run games in Gamescope" (off by default), the game is started as `gamescope <options> -- <the usual command>`. Gamescope is a separate program on your computer; nothing is sent anywhere. Kettle runs `gamescope --version` to show the version. For the overlay, a tiny `sh` shim in front of the game writes the name of Gamescope's private X display to a file in your runtime folder, and the overlay helper connects to that display (only a local socket).

## Performance overlay (MangoHud)

If you turn it on (Settings > In Game), we write a small config file per game to `mangohud/<app id>.conf` in our data folder (readable only by you) and start the game with `MANGOHUD=1` and `MANGOHUD_CONFIGFILE` set. MangoHud itself is a separate program that draws the numbers inside the game; we send nothing anywhere.

## Updates, downloads and notifications

Checking for updates asks Steam, over the shared connection, for the public build and manifest numbers of the games you have installed (the same product info requests a download makes). Nothing else is sent. Games that the official Steam client installed are only read for this check; Kettle writes into their folders only when you answer "Update this time" or "Always allow" in the dialog (Settings > Downloads, "Update games installed by the Steam client": ask / never / yes).

Desktop notifications are local: a message from Kettle to your desktop's notification service on D-Bus (`org.freedesktop.Notifications`), made by the `notify-rust` library. They are off for friends by default and can be switched off completely. The optional sound is a name from your sound theme; Kettle ships no sounds.

Shortcuts are plain `.desktop` files, written only when you ask for them (install dialog, or Properties > General): `~/.local/share/applications/kettle-game-<id>.desktop` and `<your Desktop folder>/kettle-game-<id>.desktop`. They contain the game's name and the command `kettle --play <id>`. Uninstalling the game deletes them.

Moving a game copies its folder (and its Proton prefix, so saves stay with it) into another library you chose, then deletes the old copy. "Clear cache" and "Delete local data" delete our cache folder only. Download settings (speed limit, schedule, update policy, added library folders) are in `settings.json` in our data folder.

## Known limits

- On a machine where the keyring has an empty password, the keyring is unlocked at login and its contents aren't encrypted at rest. That's the keyring's setting, not ours, but it means anyone with access to your user account can read the token.

## Steam Cloud saves

With "Steam Cloud sync" on (Settings; default on, and each game can switch it off), before a game starts Kettle asks Steam over the CM connection which save files the cloud holds for that game, downloads newer ones and, after the game ends, uploads changed ones. Sent to Steam: the app ID, the save file names (like `%WinMyDocuments%My Games/Foo/slot1.sav`), their SHA-1 and time, the file contents, and the machine name "Kettle". File contents move over HTTPS to the hosts Steam names. Nothing goes anywhere else. Kettle looks for saves only in the places the game's own Steam data lists, plus `userdata/<account>/<app>/remote` (games that use Steam's remote-storage API) but only inside a folder Kettle made itself; when an official Steam client is installed that folder belongs to it, is never written by Kettle, and is skipped (the log says so). Files we write: `~/.local/share/kettle/cloud/<app id>.json` (what was last in sync: file names, SHA-1, times), `~/.local/share/kettle/cloud_games.json` (games with Cloud off) and `~/.local/share/kettle/cloud-backups/<app id>/` (copies of local saves before a download replaced them; delete when you like). Kettle never deletes a save, locally or in the cloud.
