# Kettle's look and behaviour, measured from the reference

Source: `target/ref/steam-vs-kettle.webp` (1870 x 1079). The right half (x >= 935) is the official client
(library with the Team Fortress 2 page) at about 1:1. Colours are the most common pixel of a flat area; text
colours are the brightest pixel of the text. Positions are in the screenshot's pixels (x of the window's left
edge = 936); in Kettle they are points at 100 % zoom. Everything is flat: no gradients anywhere, because the
reference shows none (the PLAY button is flat green).

## Colours

| Surface | Value | Where measured |
|---|---|---|
| Title bar, nav row, bottom bar | `#171d24` | (1230,2)-(1420,30), (1480,36)-(1700,60), (1000,1040)-(1200,1075) |
| Window edge line | `#191c24` | x 936-940 |
| Sidebar list and page background | `#242830` | (1199,400..1025) |
| Sidebar Home strip (behind the boxes) | `#191c24` | (1003,78..80) |
| Home box, drop-down box, grid button | `#26272e` | (1000,128..135), (944,100) |
| Search row | `#25262c` | (1000,168..195) |
| Sort button | `#34373f` | (1098,130)-(1130,156) |
| Grid icon | `#4b4d51` | (1175,100) |
| Selected list row | `#3e4f69` | (1050,509)-(1190,530) |
| Hovered list row | `#313a4a` | derived: selected at half strength (not visible in the screenshot) |
| Collection header text / count | `#c6cdd5` / `#535b66` | (958,590)-(1030,604) / (1030,590)-(1075,604) |
| Row text (installed) | `#eef2f6` | (985,225)-(1100,243) |
| Menu text | `#898f96` | (985,8)-(1010,26) |
| Nav tab text / selected tab / underline | `#dee4e9` / `#2ba1f6` (`#1a9fff`) / `#269aee` | (1012,36), (1090,36), (1127,60) |
| Sidebar drop-down and Home text | `#b3b5bb` | (948,135)-(1080,152) |
| Announcements button | `#199aff` | (1440,17) |
| Bell button | `#262c35` | (1497,10) |
| Account pill | `#292e37` | (1620,17) |
| Bell, monitor, window control icons | `#899199` | (1512,17) |
| Game page play bar | `#292d32` | (1500,300)-(1700,340) |
| PLAY button | `#4bc831` | (1232,310)-(1262,340) |
| PLAY drop-down arrow segment | `#18b14c` | (1328,308)-(1346,340) |
| PLAY text | `#dfffe5` | (1268,315)-(1310,334) |
| LAST PLAYED label / value | `#a1a0a3` / `#75777b` | (1410,308), (1410,322) |
| Gear / info / heart button fill | `#3b3e44` | (1722,310)-(1752,338) |
| Gear / info / heart icon | `#a3a2a7` | (1737,316) |
| Tab box fill / border / text | `#2e333b` / `#32383f` / `#7d8087` | (1230,365)-(1830,392), (1500,357) |
| Panel (recommendation box) | `#434952` | (1230,420)-(1500,430) |
| Post-game area (dark panel) | `#202329` | (1230,555)-(1750,740) |
| Running-game icon | `#82c12a` | (945,519) |
| Featured card strip / outline / body | `#1b6192` / `#4a7a9e` / `#2a2f36` | (1240,768)-(1330,785), left edge (1229,820), body (1240,795)-(1600,860); the real card has a soft gradient, Kettle draws it flat |
| Featured "FEATURED" / kind / title text | `#e8ffff` / `#969ca3` / `#d0d3d9` | (1240,770), (1304,806), (1304,827) |
| Page section title (ACTIVITY, FRIENDS WHO PLAY) | `#a0a4a7` | (1229,906), (1650,761) |
| Friends box fill / edge / text | `#272c32` / `#3a4047` / `#cbcfd2` | (1655,790)-(1695,800), x 1650, (1662,796) |
| Friend name / rich presence line | `#ecf8e6` / `#a0b29a` | (1702,837), (1702,853) |
| Link ("View all friends who play", "View Latest News") | `#8f9498` | (1692,902)-(1815,936), (1498,987) |
| Date group header / rule above it | `#8c8e90` / `#323337` | "TODAY" (1229,1016), line y 1023 |

## Sizes and positions

| Element | Value | Where measured |
|---|---|---|
| Top bar (menu row) | y 0-33 (34 px), menu text 13 px, centre y 17 | (1019,13)-(1045,21) |
| Nav row | y 34-77 (44 px); tab text caps 13 px high (about 19 px font), centre y 46 | STORE y 40-52 |
| Tab spacing / underline | 21 px between tabs; underline 3 px, y 59-61, the width of the text | x 1090-1163 at y 60 |
| Back / forward arrows | centres x 958 and 989, y 46, 16 px | arrows y 38-53 |
| Announcements / bell / pill | x 1431-1482, 1489-1537, 1545-1725; y 5-31 | scan at y 7, 28 |
| Window controls | minimise 1789, maximise 1821, close 1852 (32 px pitch) | y 10-20 |
| Content starts | y 78 | hero top |
| Bottom bar | y 1030-1079 (49 px) | x 1000 scan |
| Sidebar | x 936-1208 (273 px); boxes x 941-1201 (5 px left, 7 px right) | row 90 scan |
| Home box / grid button | 224 x 30 at y 82-111 / 32 x 30 | (941..1165), (1169..1201) |
| Drop-down row | y 128-155 (28 px); sort button 32 px at x 1097 | col 1000 scan |
| Search row | y 168-195 (28 px) | col 1000 scan |
| Quick icons (clock, play) | centres x 1169, 1195, 14 px | row 143 scan |
| List row | pitch 26 px; selected band 24 px high (y 507-530), 1 px gap; icon 20 px at x 960; text at x 985; band ends x 1195 | col 1190 |
| Collection header | dash at x 943-953, text x 958, "(n/total)" after the title in the dim colour | crop |
| Hero | 651 x 220 | x 1209-1860, y 78-298 |
| Play bar | y 298-349; PLAY 93 x 33 at x 1229, y 308; arrow segment 24 px wide, 3 px gap | scans at y 312, x 1250 |
| LAST PLAYED / PLAY TIME | x 1411 and x 1518, 13 px | y 312 scan |
| Gear / info / heart | 32 x 32, radius 4, 10 px apart, ending x 1837 | x 1737 scan |
| Tab box | y 357-395 (38 px), x 1229-1837; texts at +23 px with 68 px between, "..." at -44 px | scans |
| Game page columns | left = rest (389), gap 32, right 187; both start 24 px below the tab box in Kettle (Steam has the recommendation box and post-game summary in between, which Kettle does not have) | x 1229-1618, 1650-1837 |
| Featured card | 389 x 100, strip 27 px, icon 36 px at x +20, kind text 13 px caps at x +75, title 19 px | y 765-865 |
| Right boxes | section title 14 px, 9 px gap, box 1 px edge radius 2, padding 11, 30 px between sections | y 761-951 |
| Radii | list rows 0, boxes and buttons 2, icon buttons 4 | zoomed crops |

## Behaviour: Steam against Kettle

| Element | Steam | Kettle |
|---|---|---|
| Game row, click | selects it, shows its page | same |
| Game row, double click / Enter | starts the game (installed), else the install dialog | same (`game_page::activate`) |
| Game row, right click | menu: Play, Add to Favorites, Add to >, Manage >, Properties | same; Manage opens as a submenu |
| Collection header click | anywhere on the row folds or unfolds it | the whole row (it used to be only the small button); right-click: Show in the library, Delete (ours) |
| Dynamic collections | filter is evaluated | the filter is not evaluated (needs store tags): only the hand-added games are listed, with a bolt badge; the old placeholder line is gone |
| Home row | opens the library home | same; the grid button does the same (there is no separate grid page yet) |
| "Games and Software" drop-down | picks the category | picks All / Installed / Ready to play |
| Sort button | sort menu | Alphabetical, reversed, Recently played, Play time |
| Clock / play icons | quick filters | clock = recent first, play = ready to play (toggle, blue when on) |
| Search | filters the list as you type | same; filter button at the right opens Installed / Ready to play / Played / Unplayed |
| PLAY | starts the game | same; arrow segment opens: Launch via Steam client, Properties |
| Gear / info / heart | Manage menu / info / favourite | same |
| Tab box links | open in the browser (Store Page in the client) | same; the first three are visible, the rest under "..." |
| Top bar tabs | click switches; hover brightens | same |
| Menu bar | click opens; moving over another title switches | same |
| Account pill | menu with profile, status, sign out | same |
| Back / forward | arrows, mouse buttons 4 and 5, Alt+arrows | same |
| Bottom bar | Add a Game menu, Downloads page, Friends & Chat panel | same (Add a Game offers product key and wallet code) |
| Escape in a drop-down or popup | closes only the popup | same: the Settings and Properties windows ignore Escape while a popup is open or a field has the keyboard |
| Escape in the main window | goes back | same, unless a menu, dialog or text field is active |

Game page, below the tab box (measured, TF2 page): recommendation box y 415-505 (#434952, texts 17 px, buttons 36 px high:
Yes 91, No 87, Maybe Later 108 wide, 6 apart, 28 from the edge); POST-GAME SUMMARY heading, then a 185 px flat panel with
224 x 126 thumbnails 22 px apart and < > X at the right of the heading; "Say something about this game" field 36 px high under
ACTIVITY, with "View Latest News" right under it. Kettle shows the recommendation box after 1 hour of play (Yes/No open the
store page, Maybe Later hides it for 30 days) and the summary from the last session's screenshots in
`~/Pictures/Kettle/<game>/`; posting from the field is not possible (no call in the client), Enter says so.

Not done on the game page: the Steam Workshop / guides / reviews strips, the Featured card's gradient (Kettle's is flat by rule). Not done: Steam's penguin (Linux only) filter icon, the wallet balance, cloud status icon, the faint logo
watermark in the Home box, store-tag based dynamic collections.

## What is tested

Automated (`crates/ui/src/tests.rs`, egui_kittest, run by `cargo test` in CI; demo library, nothing launched):

- a click on a list row selects that game (game page opens)
- a click anywhere on a collection header folds it, a second click unfolds it
- menu bar: a click opens a menu, moving over another title switches to it
- nav row: STORE tab click switches the page, the back arrow returns
- Escape while the filter popup is open closes only the popup (page and selection stay)

By eye, from the screenshots of `scripts/ci-screenshots.sh` (Xvfb, xdotool when installed; the `*-after.png`
files show the result): filter popup, row right-click menu, header fold, PLAY drop-down.

Not covered by tests: double click / Enter starting a game (would launch it), mouse buttons 4 and 5, Escape in the
separate Settings and Properties windows (they are separate viewports, which kittest cannot drive; the code path
is `chrome::escape_closes_window`), drag of the title bar, Friends & Chat panel.

## Colour comparison (Steam against Kettle, 5 x 5 averages)

`scripts/color-compare.py <kettle shot>` produces the table below from the latest CI screenshot.
| Region | Steam | Kettle | max delta |
|---|---|---|---|
| sidebar top y 124 | `#2b2f37` | `#2c3037` | 1 |
| sidebar y 200 | `#292f35` | `#272d36` | 2 |
| title bar | `#171d24` | `#171d24` | 0 |
| nav row | `#171d24` | `#171d24` | 0 |
| Home box | `#26272e` | `#26272e` | 0 |
| grid button | `#282a2f` | `#282a2f` | 0 |
| drop-down box | `#26272e` | `#26272e` | 0 |
| sort button | `#34373f` | `#34373f` | 0 |
| search row | `#25262c` | `#25262c` | 0 |
| list background | `#242830` | `#242830` | 0 |
| collection header row | `#272d36` | `#272d36` | 0 |
| play band | `#282b31` | `#282b31` | 0 |
| PLAY button | `#4dc634` | `#4dc634` | 0 |
| PLAY arrow segment | `#1ab04e` | `#18b14c` | 2 |
| gear button | `#3f3d43` | `#3d3e44` | 2 |
| tab box | `#2e3139` | `#2e333b` | 2 |
| page bg y 420 | `#282c32` | `#282c31` | 1 |
| page bg y 700 | `#24272c` | `#24272b` | 1 |
| page bg y 1000 | `#1c1e1f` | `#1c1e20` | 1 |
| divider | `#1f2326` | `#1f2227` | 1 |
| bottom bar | `#181e24` | `#171d24` | 1 |
| announcements button | `#199aff` | `#199aff` | 0 |
| bell button | `#272d36` | `#262c35` | 1 |
| account chip | `#272d35` | `#292e37` | 2 |
| nav STORE (text) | `#dee4e9` | `#dee4e9` | 0 |
| menu text (text) | `#898f96` | `#898f96` | 0 |
| Home text (text) | `#b4b5bc` | `#b3b5bb` | 1 |
| row text (text) | `#f6fbff` | `#f6fbff` | 0 |
| header text (text) | `#dbe6f4` | `#dbe6f4` | 0 |
| LAST PLAYED label (text) | `#a1a0a3` | `#a1a0a3` | 0 |

All sampled pairs are within 2 per channel (latest CI run). Unmatched by design: the selected row (not in the Kettle shot; uses the measured #3e4f69), hover colours (not visible in the reference).
