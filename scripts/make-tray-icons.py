#!/usr/bin/env python3
"""Make the monochrome system tray icons from the app badge.

Keeps only the badge's dark line art, drops the white tile and its rim,
thickens the strokes so they survive at panel sizes, then writes white
glyphs on transparency. The app recolors them to the panel's text color.
Needs python3-gi with GdkPixbuf.
"""
from pathlib import Path

import gi
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import GdkPixbuf, GLib

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'crates/entanglo-linux/data/icons/hicolor/256x256/apps/com.paoloasara.Entanglo.png'
OUT = ROOT / 'crates/entanglo-linux/data/icons/tray'
LIGHT, DARK = 110.0, 40.0  # luminance: at or above LIGHT is background, at or below DARK is full ink


def ink_map(path):
    pb = GdkPixbuf.Pixbuf.new_from_file(str(path))
    if not pb.get_has_alpha():
        pb = pb.add_alpha(False, 0, 0, 0)
    w, h, stride, px = pb.get_width(), pb.get_height(), pb.get_rowstride(), pb.get_pixels()
    ink = []
    for y in range(h):
        row = []
        for x in range(w):
            o = y * stride + x * 4
            r, g, b, a = px[o:o + 4]
            lum = 0.299 * r + 0.587 * g + 0.114 * b
            row.append(max(0.0, min(1.0, (LIGHT - lum) / (LIGHT - DARK))) * a / 255)
        ink.append(row)
    return ink, w, h


def dilate(ink, w, h, r):
    rows = [[max(row[max(0, x - r):x + r + 1]) for x in range(w)] for row in ink]
    return [[max(rows[yy][x] for yy in range(max(0, y - r), min(h, y + r + 1))) for x in range(w)] for y in range(h)]


def glyph(ink, w, h, size):
    points = [(x, y) for y in range(h) for x in range(w) if ink[y][x] > 0.1]
    x0, x1 = min(p[0] for p in points), max(p[0] for p in points)
    y0, y1 = min(p[1] for p in points), max(p[1] for p in points)
    side = max(x1 - x0, y1 - y0) + 1
    side += 2 * (side // 16)
    ox, oy = (x0 + x1) // 2 - side // 2, (y0 + y1) // 2 - side // 2
    data = bytearray(side * side * 4)
    for y in range(side):
        for x in range(side):
            sx, sy = ox + x, oy + y
            value = ink[sy][sx] if 0 <= sx < w and 0 <= sy < h else 0.0
            data[(y * side + x) * 4:(y * side + x) * 4 + 4] = bytes((255, 255, 255, round(value * 255)))
    pb = GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(bytes(data)), GdkPixbuf.Colorspace.RGB, True, 8, side, side, side * 4)
    return pb.scale_simple(size, size, GdkPixbuf.InterpType.HYPER)


ink, w, h = ink_map(SOURCE)
OUT.mkdir(parents=True, exist_ok=True)
for size, grow in ((22, 4), (32, 3), (48, 3), (64, 2)):
    glyph(dilate(ink, w, h, grow), w, h, size).savev(str(OUT / f'tray-{size}.png'), 'png', [], [])
    print(OUT / f'tray-{size}.png')
