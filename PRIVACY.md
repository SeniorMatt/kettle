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
| `api.steampowered.com` | QR sign-in and the CM server list. Also the public store lookup `IStoreBrowseService/GetItems` (no login; only the app ID) to find cover art and icons that live at hashed paths |
| Steam CM servers (`*.steamserver.net`) | The client connection (encrypted): your owned games, downloads and short-lived access tokens. After sign-in it also carries Friends & Chat: your friends list, their names and presence, your own online state (Online by default; you can pick Away, Busy, Invisible or Offline in the panel) and chat messages |
| `shared.steamstatic.com` | Cover art (only the app ID is sent) |
| `media.steampowered.com` | Small game icons |
| `store.steampowered.com` | Public store data for the Store tab: featured lists, search and app details. No login or cookies; only the country (`us`) and language (`english`) are sent, plus the search text or app ID you look at |
| `*.steamstatic.com` | Store screenshots and capsules (only the image URL is requested) |
| `avatars.steamstatic.com` | Friends' profile pictures for the Friends & Chat panel (only the picture's hash is in the URL; kept in the same image cache) |

All HTTPS uses rustls with certificate verification.

## Friends and chat

Chat messages and your friends list are held in memory only (at most 2000 friends and 500 messages per chat) and are never written to disk. Messages go to Steam over the CM connection, the same way the official client sends them. Only avatar pictures are cached on disk (see above).

## Launch options

When you have not set launch options for a game, we show and use the ones the official Steam client saved for it. They are read, never written, from `userdata/<account id>/config/localconfig.vdf` in your Steam folder, and only for the signed-in account.

## In-game overlay

The Shift+Tab shortcut is registered only while a game started here is running (and the setting is on); nothing is sent anywhere. Notes are saved per game in `notes.json` in our data folder.

## Known limits

- On a machine where the keyring has an empty password, the keyring is unlocked at login and its contents aren't encrypted at rest. That's the keyring's setting, not ours, but it means anyone with access to your user account can read the token.
