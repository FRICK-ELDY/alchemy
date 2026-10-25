#!/usr/bin/env bash
# Fetch Material Symbols Outlined "home" (Fill) from Google Fonts Icons → home.png.
#
# Source: https://fonts.google.com/icons?selected=Material+Symbols+Outlined:home:FILL@1
# SVG:    …/materialsymbolsoutlined/home/fill1/48px.svg
# License: Apache-2.0 (Material Symbols)
#
# Requires: curl, python3; downloads resvg (linux x86_64) into a temp dir.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/client/app/assets/icons/home.png"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

SVG_URL="https://fonts.gstatic.com/s/i/short-term/release/materialsymbolsoutlined/home/fill1/48px.svg"
curl -fsSL "$SVG_URL" -o "$TMP/home_raw.svg"

# Material Symbols viewBox is "0 -960 960 960". Normalize so resvg scales the glyph fully.
python3 - <<'PY' "$TMP/home_raw.svg" "$TMP/home.svg"
from pathlib import Path
import re
import sys

raw = Path(sys.argv[1]).read_text(encoding="utf-8")
m = re.search(r'\bd="([^"]+)"', raw)
if not m:
    raise SystemExit("no path in SVG")
d = m.group(1)
svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 960 960">
  <g transform="translate(0, 960)" fill="#ffffff">
    <path d="{d}"/>
  </g>
</svg>
'''
Path(sys.argv[2]).write_text(svg, encoding="utf-8")
print("svg ready")
PY

RESVG_URL="https://github.com/linebender/resvg/releases/download/v0.49.0/resvg-linux-x86_64.tar.gz"
curl -fsSL "$RESVG_URL" -o "$TMP/resvg.tgz"
tar -xzf "$TMP/resvg.tgz" -C "$TMP"
RESVG="$(find "$TMP" -type f -name resvg | head -n1)"
"$RESVG" -w 48 -h 48 "$TMP/home.svg" "$TMP/home.png"

python3 - <<'PY' "$TMP/home.png" "$OUT"
"""Decode PNG (with scanline filters) → hard white silhouette like settings/power."""
from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path


def paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def read_rgba(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos = 8
    width = height = bit_depth = color_type = None
    idat = b""
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        tag = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if tag == b"IHDR":
            width, height, bit_depth, color_type, *_ = struct.unpack(">IIBBBBB", chunk)
        elif tag == b"IDAT":
            idat += chunk
        elif tag == b"IEND":
            break
    assert bit_depth == 8 and color_type == 6 and width and height
    bpp = 4
    stride = width * bpp
    raw = zlib.decompress(idat)
    out = bytearray(height * stride)
    i = 0
    prev = bytearray(stride)
    for y in range(height):
        filt = raw[i]
        i += 1
        row = bytearray(raw[i : i + stride])
        i += stride
        if filt == 0:
            pass
        elif filt == 1:  # Sub
            for x in range(stride):
                left = row[x - bpp] if x >= bpp else 0
                row[x] = (row[x] + left) & 255
        elif filt == 2:  # Up
            for x in range(stride):
                row[x] = (row[x] + prev[x]) & 255
        elif filt == 3:  # Average
            for x in range(stride):
                left = row[x - bpp] if x >= bpp else 0
                row[x] = (row[x] + ((left + prev[x]) // 2)) & 255
        elif filt == 4:  # Paeth
            for x in range(stride):
                left = row[x - bpp] if x >= bpp else 0
                up = prev[x]
                up_left = prev[x - bpp] if x >= bpp else 0
                row[x] = (row[x] + paeth(left, up, up_left)) & 255
        else:
            raise SystemExit(f"unsupported PNG filter {filt}")
        out[y * stride : (y + 1) * stride] = row
        prev = row
    return width, height, bytes(out)


def write_png(path: Path, w: int, h: int, rgba: bytes) -> None:
    def chunk(tag: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + tag
            + payload
            + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + rgba[y * w * 4 : (y + 1) * w * 4] for y in range(h))
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


w, h, rgba_in = read_rgba(Path(sys.argv[1]))
rgba = bytearray(w * h * 4)
solid = 0
for i in range(0, len(rgba_in), 4):
    a = rgba_in[i + 3]
    # Hard alpha — match settings/power crisp silhouettes.
    a = 255 if a >= 64 else 0
    if a:
        solid += 1
    rgba[i : i + 4] = bytes([255, 255, 255, a])
write_png(Path(sys.argv[2]), w, h, bytes(rgba))
print(f"wrote {sys.argv[2]} ({w}x{h}, solid={solid}, {Path(sys.argv[2]).stat().st_size} bytes)")
PY
