# Privacy and security

steam-rust has no telemetry, no analytics, no crash reporting and no servers of its own. It only talks to Valve's servers, the same ones the official client uses.

## Signing in

- **QR code only.** You approve the sign-in in the Steam Mobile App, so a password is never typed into, seen by, or stored by this app.
- **The refresh token** (it keeps you signed in) is stored only in your system keyring: GNOME Keyring, KWallet or KeePassXC, through the Secret Service API. It's never written to a file, a log, the screen or `Debug` output.
- **Access tokens** are short-lived. They're requested when needed and kept in memory only.
- When a session ends, tokens are overwritten in memory.
- **Sign out** does three things. It deletes the token from the keyring. It revokes the token on Steam's servers, so any copy anywhere stops working. And it deletes the cached library of that account.
- **Error messages are scrubbed:** anything that looks like a token is replaced with `[redacted]` before it's shown.
- The device name sent to Steam is just `steam-rust (Linux)`. It doesn't include your hostname or username.

## Files we write

All files are readable only by you: folders are mode `0700` and files `0600`.

| Path | Contents |
|---|---|
| `~/.local/share/steam-rust/accounts.json` | Account names and SteamIDs you've signed in with (no secrets) |
| `~/.cache/steam-rust/library-<steamid>.json` | Your owned-games list, for instant startup |
| `~/.cache/steam-rust/art/` | Cover art downloaded from Steam's CDN |

We **never write** to the official Steam client's files. We only read its `steamapps/*.acf`, `libraryfolders.vdf` and `appcache/librarycache`.

## Network connections

| Host | Why |
|---|---|
| `api.steampowered.com` | QR sign-in, token refresh and revoke, owned games, and the CM server list |
| Steam CM servers (`*.steamserver.net`) | The client connection (encrypted) |
| `shared.steamstatic.com` | Cover art (only the app ID is sent) |
| `media.steampowered.com` | Small game icons |

All HTTPS uses rustls with certificate verification.

## Known limits

- On a machine where the keyring has an empty password, the keyring is unlocked at login and its contents aren't encrypted at rest. That's the keyring's setting, not ours, but it means anyone with access to your user account can read the token.
- `GetOwnedGames` takes the access token in the URL query (it's a GET API). That only travels over TLS to Valve, and it's scrubbed from any error text.
