"""Generate the synthetic sample covers used by the QML preview (qml/mock only).

Two abstract 600 px covers (teal and green, echoing referencias/ref_01 and ref_06) plus
their pre-blurred 96 px backgrounds, the way the Rust services crate will deliver them.
Standard library only (zlib + struct for PNG). Run from anywhere:

    python qml/mock/assets/make_covers.py

It also prints the worst-case contrast of the default cover ink over each blurred
background composited with the veil, so the mock palettes stay >= 4.5:1.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SIZE = 600
BLUR_SIZE = 96


def write_png(path, width, height, pixels):
    """pixels: list of rows, each a list of (r, g, b) ints."""
    raw = bytearray()
    for row in pixels:
        raw.append(0)  # filter: none
        for r, g, b in row:
            raw += bytes((r, g, b))

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def hexrgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255.0 for i in (0, 2, 4))


def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def clamp01(x):
    return 0.0 if x < 0 else 1.0 if x > 1 else x


def cover_teal(x, y):
    """Night sea: gradient sky, pale moon, layered sine swells."""
    u, v = x / SIZE, y / SIZE
    col = mix(hexrgb("#0E2636"), hexrgb("#2B7F98"), clamp01(v * 1.1))
    # moon with a soft halo
    d = math.hypot(u - 0.70, v - 0.27)
    col = mix(col, hexrgb("#4FA7BD"), clamp01((0.26 - d) / 0.12) * 0.55)
    col = mix(col, hexrgb("#C6E8EF"), clamp01((0.115 - d) * SIZE + 0.5))
    # swells, back to front; 1 px antialiased edges
    swells = [
        (0.56, 0.030, 2.0, 0.3, "#1F5F78"),
        (0.66, 0.035, 2.6, 1.7, "#3D9DB4"),
        (0.76, 0.028, 3.4, 0.9, "#164B60"),
        (0.86, 0.032, 2.2, 2.4, "#0B3142"),
    ]
    for base, amp, freq, phase, hx in swells:
        edge = base + amp * math.sin(u * freq * math.tau + phase)
        col = mix(col, hexrgb(hx), clamp01((v - edge) * SIZE + 0.5))
    return col


def cover_green(x, y):
    """Four-panel collage of concentric rings in greens (an abstract ref_06)."""
    gap = 10
    half = SIZE / 2
    if abs(x - half) < gap / 2 or abs(y - half) < gap / 2:
        return hexrgb("#07150E")
    qx, qy = int(x >= half), int(y >= half)
    cx = half * qx + half / 2 + (-18 if qx else 22)
    cy = half * qy + half / 2 + (16 if qy else -20)
    tiles = [
        ("#0F3A26", "#5FD39A", 26.0),
        ("#123020", "#2E8B5E", 18.0),
        ("#0B2A1B", "#3FB57A", 34.0),
        ("#174A31", "#8BE8B8", 22.0),
    ]
    dark, light, period = tiles[qy * 2 + qx]
    d = math.hypot(x - cx, y - cy)
    ring = 0.5 + 0.5 * math.cos(d / period * math.tau)
    fall = clamp01(1.0 - d / (half * 0.95))
    col = mix(hexrgb(dark), hexrgb(light), ring * fall * 0.85)
    # small solid core
    return mix(col, hexrgb("#E6FBEF"), clamp01((14 - d) + 0.5) * 0.9)


def render(fn):
    return [[fn(x + 0.5, y + 0.5) for x in range(SIZE)] for y in range(SIZE)]


def to_bytes(img):
    return [[tuple(int(round(clamp01(c) * 255)) for c in px) for px in row] for row in img]


def downsample(img, n):
    step = SIZE / n
    out = []
    for j in range(n):
        y0, y1 = int(j * step), int((j + 1) * step)
        row = []
        for i in range(n):
            x0, x1 = int(i * step), int((i + 1) * step)
            acc = [0.0, 0.0, 0.0]
            cnt = 0
            for yy in range(y0, y1):
                for xx in range(x0, x1):
                    p = img[yy][xx]
                    acc[0] += p[0]
                    acc[1] += p[1]
                    acc[2] += p[2]
                    cnt += 1
            row.append(tuple(a / cnt for a in acc))
        out.append(row)
    return out


def box_blur(img, r, passes):
    n = len(img)
    for _ in range(passes):
        for horizontal in (True, False):
            nxt = [[None] * n for _ in range(n)]
            for a in range(n):
                for b in range(n):
                    acc = [0.0, 0.0, 0.0]
                    for k in range(-r, r + 1):
                        c = min(n - 1, max(0, b + k))
                        p = img[a][c] if horizontal else img[c][a]
                        acc[0] += p[0]
                        acc[1] += p[1]
                        acc[2] += p[2]
                    v = tuple(x / (2 * r + 1) for x in acc)
                    if horizontal:
                        nxt[a][b] = v
                    else:
                        nxt[b][a] = v
            img = nxt
    return img


def lin(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lum(rgb):
    r, g, b = (lin(c) for c in rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = lum(a), lum(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


# Palettes mirrored in qml/preview/NowPlayingPreview.qml (veil opacity from Theme.cover).
PALETTES = {
    "teal": {"veilDark": "#061B24", "veilLight": "#D7E8EE", "inkDark": "#F4FAFC", "inkLight": "#0D2129"},
    "green": {"veilDark": "#12301F", "veilLight": "#D9EEE1", "inkDark": "#F3FBF6", "inkLight": "#0C2417"},
}
VEIL_OPACITY = {"dark": 0.52, "light": 0.58}


def worst_contrast(blur, pal, mode):
    veil = hexrgb(pal["veilDark" if mode == "dark" else "veilLight"])
    ink = hexrgb(pal["inkDark" if mode == "dark" else "inkLight"])
    a = VEIL_OPACITY[mode]
    worst = 99.0
    for row in blur:
        for p in row:
            comp = mix(p, veil, a)
            worst = min(worst, contrast(comp, ink))
    return worst


def main():
    for name, fn in (("teal", cover_teal), ("green", cover_green)):
        img = render(fn)
        write_png(os.path.join(HERE, "cover_%s.png" % name), SIZE, SIZE, to_bytes(img))
        blur = box_blur(downsample(img, BLUR_SIZE), 5, 3)
        write_png(os.path.join(HERE, "cover_%s_blur.png" % name), BLUR_SIZE, BLUR_SIZE, to_bytes(blur))
        pal = PALETTES[name]
        print("%s: worst ink contrast dark %.2f:1, light %.2f:1"
              % (name, worst_contrast(blur, pal, "dark"), worst_contrast(blur, pal, "light")))


if __name__ == "__main__":
    main()
