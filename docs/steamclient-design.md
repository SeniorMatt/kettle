# Kettle's own steamclient (games without the official Steam client)

Goal: a game that uses the Steamworks API runs with only Kettle installed, signed in as the user's real account.
This is a client implementation, not a crack: ownership, tickets and stats come from Steam's servers for the
signed-in account. Nothing here fakes ownership, patches or strips DRM, or lets an unowned game start.

Status: milestone 1 (core in `crates/steamclient`, host in `crates/client/src/steamworks_host.rs`) is written and
unit-tested on paper only until the machine is free to build (see "Verification status"). Everything marked
"to verify" below must be checked against Valve's published headers or a real game before it is relied on.

## 1. How a game reaches the client

```
game ── libsteam_api.so (shipped with the game, links nothing of Valve's client)
          │ dlopen("…/steamclient.so"), dlsym("CreateInterface"), "Steam_BGetCallback", …
          ▼
        steamclient.so  ◄── this is what Kettle provides
          │ C++ objects: ISteamClient → GetISteamUser/Apps/Utils/UserStats/Friends/…
          ▼
        IPC to the real client process (official client: shared memory + a pipe; Kettle: a Unix socket)
```

- `SteamAPI_Init` loads `steamclient.so`, calls `CreateInterface("SteamClientNNN")`, then through that object
  `CreateSteamPipe`, `ConnectToGlobalUser`, and `GetISteamXxx(user, pipe, "VersionString")` for each interface the
  game's compiled-in header version asks for. The game never gets an interface it did not name by version.
- `SteamAPI_RunCallbacks` drains `Steam_BGetCallback(pipe, &CallbackMsg_t, &call)` / `Steam_FreeLastCallback(pipe)`.
  A `SteamAPICallCompleted_t` (id 703) announces a call result; the game then fetches it with
  `Steam_GetAPICallResult(pipe, call, buf, size, expectedCallbackId, &failed)`.
- Windows games under Proton do not load our library directly. Proton's `lsteamclient` (a Wine builtin) forwards
  every call to the Linux `steamclient.so`; it wraps each interface version in a translation layer (it carries
  per-version wrappers: SteamClient006–021, SteamUser…, SteamUserStats…, and many more), so the Linux library must
  offer every version the game asks for.

## 2. Where the libraries are found (Proton, confirmed in source)

Proton 9.0 `lsteamclient/unixlib.cpp` (`steamclient_init`): on x86-64 it does
`dlopen("$HOME/.steam/sdk64/steamclient.so", RTLD_NOW)` after `realpath` (i386: `.steam/sdk32/steamclient.so`),
then `dlsym`s seven functions and fails the whole start if any is missing: `CreateInterface`, `Steam_BGetCallback`,
`Steam_GetAPICallResult`, `Steam_FreeLastCallback`, `Steam_ReleaseThreadLocalMemory`, `Steam_IsKnownInterface`,
`Steam_NotifyMissingInterface`. (The `STEAM_COMPAT_CLIENT_INSTALL_PATH` variable is only used on macOS.) It also
reads `SteamAppId` and `IgnoreChildProcesses` from the Windows environment. The Breakpad_* exports of
`lsteamclient` only trace; our library exports harmless versions too.

Native `libsteam_api.so` (to verify with `strace -f -e trace=openat` on a real game during the install milestone):
expected to use `~/.steam/sdk64/steamclient.so` too, plus `~/.steam/steam.pid` / `~/.steam/registry.vdf`
(`ActiveProcess` keys: `pid`, `SteamClientDll64`, `ActiveUser`) to decide whether "Steam is running" and which
account is active. Environment the game sees: `SteamAppId`, `SteamGameId`, `SteamOverlayGameId` (all set by the
launcher), and a `steam_appid.txt` next to the executable for apps started outside Steam.

## 3. ABI (what the hand-written tables must match)

- An interface object is a pointer to a pointer to a table of function pointers. Slot n is the n-th virtual
  function in header order; overloads sit next to each other in declaration order (GCC/Itanium). The Steamworks
  interfaces have no virtual destructor and no data members visible to the caller.
- x86-64 System V: `this` in `rdi`, integers/pointers in `rsi rdx rcx r8 r9`, floats in `xmm0..`, `bool` in `al`,
  `CSteamID` and other 8-byte trivially copyable values by value in a register, returned in `rax`.
  i386: cdecl-like with `this` pushed first (no callee cleanup), so a plain `extern "C" fn(this, …)` matches. A 32-bit
  game needs a 32-bit `steamclient.so` (`i686-unknown-linux-gnu` build of the same crate, later).
- Callback structures on Linux are packed to 4 bytes (`VALVE_CALLBACK_PACK_SMALL`), so e.g. `UserStatsReceived_t`
  is `{u64 game; i32 result; u64 steamid}` = 20 bytes with the steam id at offset 12. The library builds these as
  byte vectors with explicit offsets. `CallbackMsg_t` is `{i32 user; i32 callback; u8* param; i32 size}`.
- A slot we do not implement points at `zero()`, which returns 0/false/NULL for any signature; string getters we do
  not implement return `""`, never NULL. Unknown interface versions get a table of 160 such slots plus a log line
  naming the version, which is how we learn what games ask for. Every table has 24 spare zero slots at the end.

### Interface versions

Implemented (slot order lives in `crates/steamclient/src/iface.rs`; each builder lists slots in header order):

| Interface | Version strings accepted | Notes |
|---|---|---|
| ISteamClient | SteamClient017..021 | all `GetISteamXxx(user,pipe,ver)` slots share one function that looks at the version string, so a wrong guess about a later slot's position does not misroute them; `GetISteamUtils(pipe,ver)` has its own. To verify: position of `GetISteamGameSearch` (assumed inserted at slot 19 from 020) |
| ISteamUser | SteamUser019..023 | GetSteamID, BLoggedOn, GetAuthSessionTicket, BeginAuthSession (answers "invalid ticket": we do not validate other players), CancelAuthTicket, UserHasLicenseForApp, RequestEncryptedAppTicket/GetEncryptedAppTicket. To verify: 023 inserts `GetAuthTicketForWebApi` after `GetAuthSessionTicket` |
| ISteamApps | STEAMAPPS_INTERFACE_VERSION006..008 | BIsSubscribed, BIsSubscribedApp, BIsDlcInstalled (= owned), GetCurrentGameLanguage, GetAppInstallDir, BIsAppInstalled, GetAppOwner, GetAppBuildId, GetLaunchCommandLine |
| ISteamUtils | SteamUtils009..010 | GetAppID, GetServerRealTime, GetConnectedUniverse, IsAPICallCompleted/GetAPICallResult, IsOverlayEnabled = false, GetSteamUILanguage |
| ISteamUserStats | STEAMUSERSTATS_INTERFACE_VERSION011..013 | RequestCurrentStats, Get/SetStat (int, float), Get/Set/ClearAchievement, GetAchievementAndUnlockTime, StoreStats, ResetAllStats; callbacks 1101/1102/1103 |
| ISteamFriends | SteamFriends015..018 | GetPersonaName, GetPersonaState, own persona via GetFriendPersonaName; friend count 0 |

Everything else (Remote storage, Networking*, Matchmaking, Input/Controller, UGC, HTTP, Inventory, …) is the null object
for now. The layouts were written from the published SDK headers' declaration order and are not machine-checked yet;
the first thing to do on a free machine is to diff each table against the SDK's `isteam*.h` slot by slot (read the
headers, do not copy them into the repo).

## 4. Architecture

Two processes, one Unix socket:

```
game ─► steamclient.so (crates/steamclient, cdylib) ──JSON lines──► Kettle (crates/client/src/steamworks_host.rs)
                                                       $XDG_RUNTIME_DIR/kettle-steamclient.sock   │
                                                                                                  ▼ CM session (cm_hub)
```

- `crates/steamclient` (`sr-steamclient`, crate-type `cdylib` + `rlib`, lib name `steamclient` so the artifact is
  `libsteamclient.so`, installed under the name `steamclient.so`). Feature `exports` (default) adds the C exports;
  Kettle depends on the rlib without it to share `proto` (the wire types) and `ipc` (the client).
- Protocol: one JSON object per line, `{"id":N,"req":{"op":…}}` → `{"id":N,"rep":{"kind":…}}`, lines ≤ 1 MiB, bytes as
  hex. Ops: `hello{app}` (first; names the app), `owns`, `app_ticket`, `auth_ticket`, `encrypted_ticket`,
  `get_stats`, `store_stats`, `app_info`. The game side asks lazily and caches (identity, ownership, app info,
  stats); if Kettle cannot be reached the library reports "not logged on" and owns nothing.
- Host rules (`handle()`): socket mode 0600 and `SO_PEERCRED` same-user check; a connection must say `hello` first and
  is then served for that app only (ownership and install-info lookups for other app ids, such as DLC, are allowed);
  tickets and stats require `owns(app)` from the account's own owned-games data and a live CM session, else
  `EResult 15` / `3`; names ≤ 256 bytes and ≤ 4096 entries; at most 16 connections.
- Where each answer comes from:

| Question | Source |
|---|---|
| SteamID, logged on | the CM connection's session (`Connection::steam_id`, hub online state) |
| BIsSubscribedApp | the owned-games list fetched over the CM (`Player.GetOwnedGames`), set with `CmBackend::set_owned` |
| Ownership ticket | `CMsgClientGetAppOwnershipTicket` (EMsg 857/858) over the CM |
| Encrypted app ticket | `CMsgClientRequestEncryptedAppTicket` (EMsg 5526/5527); the reply's `EncryptedAppTicket.encrypted_ticket` bytes go to the game (to verify against a real game: whether the client hands out these bytes or the whole serialized message) |
| Auth session ticket | currently the raw ownership ticket (below) |
| Stats, achievements | stored locally per account and app (`<data>/steamworks/<steamid>/<app>.json`), **not yet synced to Steam** |
| Persona name, language, install dir, build id | Kettle's own state (setters on `CmBackend`) |

The proto crate (`steam-vent-proto-steam` 0.5.2) has the messages but does not tag the ownership/encrypted-ticket
requests and responses with their `EMsg` kind, so the host wraps them in small local newtypes that do.

### Known gaps, stated plainly

- **Auth session tickets** (`GetAuthSessionTicket`, used for multiplayer authentication) are framed by the official
  client as GC token + session header (steam id, IPs, timestamps) + ownership ticket. We return the bare ownership
  ticket, which a server that verifies the full framing will reject. Needs the GC-token flow (`CMsgClientAuthList`
  / game connect tokens) and the framing, researched on a real game.
- **Stats sync**: stat and achievement *names* map to ids and bits through the app's stats schema (binary KeyValues in
  `CMsgClientGetUserStatsResponse.schema`); `CMsgClientGetUserStats` / `CMsgClientStoreUserStats2` carry numeric ids
  and achievement unlock-time blocks. Until the schema parser exists the values stay on this machine.
- **SteamStub DRM**: the stub wrapper in protected executables asks the client for proof of ownership at start-up;
  Proton ships a wrapper for `ISteamAppTicket` (`STEAMAPPTICKET_INTERFACE_VERSION001`), which is the interface that
  returns an ownership ticket to such callers. We have the real ticket (`app_ticket`) but have not implemented that
  interface or confirmed the exact calls; to be established by logging the unknown-interface lines on a real owned
  game. We only ever hand out a ticket Steam issued for the signed-in account, so an unowned game gets none.
- Cloud (`Cloud.*` service methods), rich presence, leaderboards, lobbies, networking, Steam Input, workshop, overlay,
  voice are not implemented (null objects).

## 5. Installing it so games find it without Steam (next milestone, not built)

Only when no official client is installed: detect it (a `~/.steam/steam` or `~/.local/share/Steam` that is not
ours, a `steam` binary on `PATH`, Flatpak `com.valvesoftware.Steam`) and **refuse** otherwise; never overwrite or
redirect its links. If clear: create a Kettle-owned root (`~/.local/share/kettle/steamroot/` with `linux64/steamclient.so`,
`linux32/steamclient.so`, `steam.sh`-less layout, `config/`, `steamapps` symlink to the library), then
`~/.steam/sdk64/steamclient.so` → our file, `~/.steam/steam` → the root, `~/.steam/steam.pid` / `registry.vdf`
with Kettle's pid while it runs. `play.rs` also sets `SteamAppId`, `SteamGameId`, `KETTLE_STEAMCLIENT_SOCK` (or relies on
`XDG_RUNTIME_DIR`), and `STEAM_COMPAT_CLIENT_INSTALL_PATH` to the root for Proton. Kettle must have started the host
(`Server::start`) with a `CmBackend` before the game starts, and updated `set_owned` / `set_persona`.

## 6. References and licences

- Valve Steamworks SDK headers (`isteam*.h`, `steam_api_flat.h`): the authoritative definition of interface layouts and
  callback structures. Their licence forbids redistribution outside the SDK, so nothing is copied; our tables are our
  own code. Version strings and slot orders come from knowledge of those headers and still need the slot-by-slot check.
- Proton `lsteamclient` (ValveSoftware/Proton, `proton_9.0`; it carries its own LICENSE file, treated as copyleft):
  read for facts only (library lookup path, required exports, the list of interface versions wrapped). No code used.
- Goldberg Steam Emulator and similar open reimplementations (LGPL): not used. Their goal (running without owning the
  game) is exactly what Kettle must not do.
- `steam-vent` 0.5 / `steam-vent-proto-steam` 0.5.2 (MIT): the CM client and the protobuf types.

## 7. Verification status

Written without a compiler because the machine was busy with a game. Before anything else: `cargo build -p
sr-steamclient`, `cargo test -p sr-steamclient` (proto round trips; `tests/dlopen.rs` loads the built
`libsteamclient.so`, creates `SteamClient021`, walks User/Apps/Utils/Friends/UserStats through the tables, callbacks
and call results, against a fake host), `cargo test -p sr-client steamworks_host` (request rules, stats file, socket).
