# steam-rust (working name)

A fast, native Steam client written in Rust: egui + wgpu, with no Chromium and no web views.

> **Unofficial.** This project is not affiliated with, endorsed by, or sponsored by Valve Corporation. "Steam" is a trademark of Valve Corporation. The working name will change before any public release.

## Status

Early development. What works so far:

- QR-code login with the Steam mobile app. No password is ever entered or stored, and the refresh token is kept in the system keyring (Secret Service: GNOME Keyring, KWallet, KeePassXC).
- Owned games through the Steam Web API.
- Detecting games already installed by the official client: Debian/Ubuntu package, native `~/.local/share/Steam`, Flatpak and Snap layouts.

## Build

```
cargo build --release
```

Developer CLI:

| Command | What it does |
|---|---|
| `sr-cli local` | Lists games installed by an existing Steam install |
| `sr-cli login` | Starts a QR login. The QR is written to `~/.cache/steam-rust/qr.svg` |
| `sr-cli owned <account>` | Counts the account's owned games |
| `sr-cli cm <account>` | Logs on to Steam's CM servers and shows the license count |
| `sr-cli logout <account>` | Removes the stored token |

## Design

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the code is organized, and [REFERENCES.md](REFERENCES.md) for the projects studied.

Game art is loaded at runtime from the local Steam cache or Steam's CDN, and is never part of this repository. The UI uses the Figtree and Inter fonts (OFL-1.1) and Phosphor icons (MIT).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
