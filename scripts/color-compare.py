#!/usr/bin/env python3
"""Colour comparison of a Kettle screenshot against the Steam half of target/ref/steam-vs-kettle.webp.
Usage: scripts/color-compare.py <kettle shot 935 wide> [markdown out]
Each flat-area pair is the average of a 5 x 5 patch; "text" pairs compare the brightest pixel of a box.
Steam points are in the reference's coordinates, Kettle points in the shot's (Steam's window starts at x 936)."""
import sys
from PIL import Image

ref = Image.open('target/ref/steam-vs-kettle.webp').convert('RGB')
k = Image.open(sys.argv[1]).convert('RGB')

def avg(im, x, y, r=2):
    px = [im.getpixel((x + dx, y + dy)) for dx in range(-r, r + 1) for dy in range(-r, r + 1)]
    return tuple(round(sum(p[i] for p in px) / len(px)) for i in range(3))

def bright(im, box):
    px = list(im.crop(box).getdata())
    return max(px, key=sum)

hexv = lambda c: '#%02x%02x%02x' % c
FLAT = [  # name, steam (x,y), kettle (x,y)
 ('sidebar top y 124', (1000, 124), (64, 124)), ('sidebar y 200', (1190, 200), (254, 200)), ('title bar', (1300, 12), (364, 12)), ('nav row', (1700, 66), (764, 66)),
 ('Home box', (1100, 92), (164, 92)), ('grid button', (1185, 108), (249, 108)),
 ('drop-down box', (1050, 153), (114, 153)), ('sort button', (1105, 150), (169, 150)),
 ('search row', (1060, 186), (124, 186)), ('list background', (1180, 986), (244, 990)),
 ('collection header row', (1100, 597), (200, 216)),
 ('play band', (1650, 335), (714, 335)), ('PLAY button', (1232, 318), (300, 318)),
 ('PLAY arrow segment', (1328, 312), (392, 318)), ('gear button', (1724, 314), (800, 314)),
 ('tab box', (1750, 376), (750, 380)), ('page bg y 420', (1215, 420), (282, 420)),
 ('page bg y 700', (1215, 700), (282, 700)), ('page bg y 1000', (1215, 1000), (282, 1000)),
 ('divider', (1207, 500), (271, 500)), ('bottom bar', (1000, 1070), (64, 1070)),
 ('announcements button', (1440, 10), (598, 10)), ('bell button', (1495, 10), (658, 10)),
 ('account chip', (1690, 9), (780, 9)),
]
TEXT = [  # name, steam box, kettle box
 ('nav STORE', (1012, 36, 1070, 56), (77, 36, 135, 56)), ('menu text', (985, 8, 1010, 26), (40, 8, 75, 26)),
 ('Home text', (948, 90, 988, 104), (12, 90, 52, 104)), ('row text', (985, 225, 1100, 243), (50, 236, 150, 250)),
 ('header text', (958, 590, 1030, 604), (23, 209, 90, 223)),
 ('LAST PLAYED label', (1410, 308, 1500, 322), (473, 313, 560, 327)),
]
rows = []
bad = 0
for name, s, kk in FLAT:
    a, b = avg(ref, *s), avg(k, *kk)
    d = max(abs(x - y) for x, y in zip(a, b)); bad += d > 2
    rows.append((name, hexv(a), hexv(b), d))
for name, s, kk in TEXT:
    a, b = bright(ref, s), bright(k, kk)
    d = max(abs(x - y) for x, y in zip(a, b)); bad += d > 2
    rows.append((name + ' (text)', hexv(a), hexv(b), d))
out = ['| Region | Steam | Kettle | max delta |', '|---|---|---|---|']
out += [f'| {n} | `{a}` | `{b}` | {d}{" **" if d > 2 else ""} |' for n, a, b, d in rows]
print('\n'.join(out)); print('differing by more than 2:', bad)
if len(sys.argv) > 2: open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
