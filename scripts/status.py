#!/usr/bin/env python3
"""Draw docs/images/status.svg (how much of Steam Kettle already does) and
write the same numbers into the README's status block.

    python3 scripts/status.py          # rewrite the picture and the README block
    python3 scripts/status.py --check  # exit 1 when either is out of date

Input is docs/roadmap.md: every `- [x]` / `- [ ]` line is one feature, and
the text before the first colon is its area. The picture is a treemap: one
box per area, sized by its number of features, filled with one cell per
feature (green when made, grey when not), biggest areas first.

Only the standard library; the output is deterministic.
"""
import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROADMAP = ROOT / "docs" / "roadmap.md"
SVG = ROOT / "docs" / "images" / "status.svg"
README = ROOT / "README.md"
START, END = "<!-- status:start -->", "<!-- status:end -->"

W, H, PAD, TOP = 900, 520, 12, 74
DONE, TODO, BACK, TEXT, DIM = "#59bf40", "#3d4450", "#171a21", "#dcdedf", "#8f98a0"
ACCENT = "#1a9fff"

ITEM = re.compile(r"^- \[([ xX])\] ([^:]+):")


def read_items():
    """[(area, done)] in file order."""
    items = []
    for line in ROADMAP.read_text(encoding="utf-8").splitlines():
        m = ITEM.match(line)
        if m:
            items.append((m.group(2).strip(), m.group(1) != " "))
    return items


def group(items):
    """{area: [done, ...]} keeping the first-seen order."""
    areas = {}
    for area, done in items:
        areas.setdefault(area, []).append(done)
    return areas


def worst_ratio(row, side):
    """Squarified treemap: the worst aspect ratio of `row` laid along `side`."""
    total = sum(row)
    thick = total / side
    return max(max(thick * thick / a, a / (thick * thick)) for a in row)


def squarify(sizes, x, y, w, h):
    """Lay `sizes` (sorted, biggest first) into the rectangle; returns rects.
    The classic algorithm: grow a row along the shorter side while the worst
    aspect ratio improves, then fix the row and continue in what's left."""
    scale = w * h / sum(sizes)
    areas = [s * scale for s in sizes]
    rects, row = [], []
    while areas:
        side = min(w, h)
        a = areas[0]
        if not row or worst_ratio(row + [a], side) <= worst_ratio(row, side):
            row.append(a)
            areas.pop(0)
            continue
        x, y, w, h = place_row(row, x, y, w, h, rects)
        row = []
    if row:
        place_row(row, x, y, w, h, rects)
    return rects


def place_row(row, x, y, w, h, rects):
    total = sum(row)
    if w >= h:  # row is a column on the left
        cw = total / h
        cy = y
        for a in row:
            rects.append((x, cy, cw, a / cw))
            cy += a / cw
        return x + cw, y, w - cw, h
    rh = total / w  # row is a strip along the top
    cx = x
    for a in row:
        rects.append((cx, y, a / rh, rh))
        cx += a / rh
    return x, y + rh, w, h - rh


def cells(n, w, h):
    """A near-square grid of n cells filling w x h: (cols, rows)."""
    best = (n, 1)
    best_score = float("inf")
    for cols in range(1, n + 1):
        rows = -(-n // cols)
        cw, ch = w / cols, h / rows
        score = max(cw / ch, ch / cw) + (cols * rows - n) * 0.3
        if score < best_score:
            best, best_score = (cols, rows), score
    return best


def pct(done, total):
    return 100.0 * done / total if total else 0.0


def draw(areas):
    total = sum(len(v) for v in areas.values())
    done = sum(sum(v) for v in areas.values())
    p = pct(done, total)
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" '
        'font-family="Inter, Segoe UI, sans-serif">',
        f'<rect width="{W}" height="{H}" rx="10" fill="{BACK}"/>',
        f'<text x="{PAD + 4}" y="34" font-size="20" font-weight="700" fill="{TEXT}">'
        f"Kettle vs. Steam: {p:.1f}%</text>",
        f'<text x="{PAD + 4}" y="56" font-size="13" fill="{DIM}">{done} of {total} features built. '
        "Green = built and tested, grey = still to do.</text>",
    ]
    # Overall progress bar on the right of the title.
    bx, bw = W - PAD - 300, 300
    out.append(f'<rect x="{bx}" y="22" width="{bw}" height="14" rx="7" fill="{TODO}"/>')
    out.append(f'<rect x="{bx}" y="22" width="{bw * p / 100:.1f}" height="14" rx="7" fill="{DONE}"/>')

    order = sorted(areas, key=lambda a: (-len(areas[a]), a))
    rects = squarify([len(areas[a]) for a in order], PAD, TOP, W - 2 * PAD, H - TOP - PAD)
    for area, (x, y, w, h) in zip(order, rects):
        flags = sorted(areas[area], reverse=True)  # made first, so the green reads as a fill
        gap = 3
        ix, iy, iw, ih = x + gap, y + gap, w - 2 * gap, h - 2 * gap
        cols, rows = cells(len(flags), iw, ih)
        cw, ch = iw / cols, ih / rows
        for i, ok in enumerate(flags):
            cx, cy = ix + (i % cols) * cw, iy + (i // cols) * ch
            out.append(
                f'<rect x="{cx + 1:.1f}" y="{cy + 1:.1f}" width="{cw - 2:.1f}" height="{ch - 2:.1f}" '
                f'rx="2" fill="{DONE if ok else TODO}"/>'
            )
        a_done = sum(flags)
        label = f"{area} {pct(a_done, len(flags)):.0f}%"
        while len(label) * 7 + 10 > iw - 4 and len(label) > 4:  # narrow box: shorten the name
            area = area[:-1]
            label = f"{area}. {pct(a_done, len(flags)):.0f}%"
        label = html.escape(label)
        out.append(f'<rect x="{ix + 2:.1f}" y="{iy + 2:.1f}" width="{len(label) * 7 + 10}" height="18" rx="3" '
                   'fill="#000" fill-opacity="0.55"/>')
        out.append(f'<text x="{ix + 7:.1f}" y="{iy + 15:.1f}" font-size="12" font-weight="600" '
                   f'fill="{TEXT}">{label}</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n", done, total


def readme_block(areas, done, total):
    rows = ["| Area | Built |", "|---|---|"]
    for area, flags in areas.items():
        n, d = len(flags), sum(flags)
        rows.append(f"| {area} | {d}/{n} ({pct(d, n):.0f}%) |")
    return "\n".join([
        START,
        f"**Kettle does {pct(done, total):.1f}% of what Steam does** ({done} of {total} features; "
        "see the [roadmap](docs/roadmap.md)).",
        "",
        "![Status](docs/images/status.svg)",
        "",
        *rows,
        END,
    ])


def main():
    check = "--check" in sys.argv
    areas = group(read_items())
    svg, done, total = draw(areas)
    readme = README.read_text(encoding="utf-8")
    block = readme_block(areas, done, total)
    if START in readme:
        new_readme = re.sub(re.escape(START) + ".*?" + re.escape(END), lambda _: block, readme, flags=re.S)
    else:
        new_readme = readme.replace("## Status\n", "## Status\n\n" + block + "\n", 1)
    old_svg = SVG.read_text(encoding="utf-8") if SVG.exists() else ""
    if check:
        stale = old_svg != svg or new_readme != readme
        print("status: out of date, run scripts/status.py" if stale else "status: up to date")
        sys.exit(1 if stale else 0)
    SVG.parent.mkdir(parents=True, exist_ok=True)
    SVG.write_text(svg, encoding="utf-8")
    README.write_text(new_readme, encoding="utf-8")
    print(f"{pct(done, total):.1f}% ({done}/{total})")


if __name__ == "__main__":
    main()
