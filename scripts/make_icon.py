"""Genera el logo de Velo: SVG de la marca, versión mono, wordmark y velo.ico.

Uso:
    python scripts/make_icon.py     # regenera velo.svg, velo_mono.svg, velo_wordmark.svg y velo.ico

Requiere Pillow; el wordmark usa fontTools (opcional: si falta, se omite).

La marca "Tajo" es una V itálica de dos astas desiguales, cortada por tres líneas
paralelas en diagonal ascendente (los dos remates y un tajo que atraviesa ambas astas).
Todo el dibujo sale de unos pocos parámetros en unidades de 64; los tamaños chicos
del .ico recalculan esos parámetros en píxeles enteros (ajuste a píxel) en vez de
escalar el vector, para que el tajo nunca baje de un píxel limpio a 16 px.
"""

from __future__ import annotations

import io
import os
import struct
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Verde Velo: esmeralda azulado (tono ~161°), más frío y oscuro que el de Spotify
# (#1DB954, ~141°). Es el verde más luminoso de ese tono que aún da >= 3:1 sobre
# blanco y sobre Theme.bg claro (#F5F7F6), y ~6:1 sobre Theme.bg oscuro (#0E0F0F).
GREEN = (0x00, 0xA8, 0x74)
GREEN_HEX = "#00A874"
# Placa del ícono de app: casi negro con un tinte verde (no se funde con la barra
# de tareas clara ni con la oscura de Windows 11, que es gris neutro).
PLATE = (0x0D, 0x1A, 0x15)
# Sobre la placa (siempre oscura) la V usa una menta más luminosa del mismo tono.
MINT = (0x2E, 0xD9, 0x9C)
MONO = (0, 0, 0)


# ---------------------------------------------------------------- geometría
def inter(p, d, q, e):
    den = d[0] * e[1] - d[1] * e[0]
    t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / den
    return (p[0] + t * d[0], p[1] + t * d[1])


def _clip(poly, a, b):
    def side(p):
        return (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])

    out = []
    for i in range(len(poly)):
        P, Q = poly[i], poly[(i + 1) % len(poly)]
        sp, sq = side(P), side(Q)
        if sp >= 0:
            out.append(P)
        if (sp >= 0) != (sq >= 0):
            t = sp / (sp - sq)
            out.append((P[0] + t * (Q[0] - P[0]), P[1] + t * (Q[1] - P[1])))
    return out


def keep(poly, a, b, ref):
    """Recorta poly al semiplano de la recta a-b que contiene ref."""
    side = (b[0] - a[0]) * (ref[1] - a[1]) - (b[1] - a[1]) * (ref[0] - a[0])
    return _clip(poly, a, b) if side >= 0 else _clip(poly, b, a)


def tidy(poly, eps=1e-6):
    out = []
    for p in poly:
        if not out or abs(p[0] - out[-1][0]) > eps or abs(p[1] - out[-1][1]) > eps:
            out.append(p)
    if len(out) > 1 and abs(out[0][0] - out[-1][0]) < eps and abs(out[0][1] - out[-1][1]) < eps:
        out.pop()
    # quita vértices colineales
    res = []
    n = len(out)
    for i in range(n):
        a, b, c = out[i - 1], out[i], out[(i + 1) % n]
        cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
        if abs(cross) > 1e-7:
            res.append(b)
    return res


# Parámetros de la marca (unidades de 64, y hacia abajo).
TAJO = dict(
    vx=27.0, vy=59.0,       # vértice inferior
    top=8.0,                # altura del remate en el borde exterior izquierdo
    dL=(-16.0, -51.0),      # dirección del asta izquierda (empinada)
    dR=(24.0, -51.0),       # dirección del asta derecha (más tendida: itálica)
    wL=15.0, wR=13.0,       # anchos horizontales de las astas
    slope=-0.16,            # pendiente de remates y tajo (sube hacia la derecha)
    cut=34.0,               # altura del tajo (borde superior) sobre el borde izq.
    gap=4.5,                # alto vertical del tajo
)


def tajo(p=TAJO, slash=True, flat_top=False):
    V0 = (p["vx"], p["vy"])
    dL, dR = p["dL"], p["dR"]
    s = (1.0, p["slope"])
    Li = (V0[0] + p["wL"], V0[1])
    Ri = (V0[0] - p["wR"], V0[1])
    C = inter(Li, dL, Ri, dR)
    tdir = (1.0, 0.0) if flat_top else s
    tp = inter(V0, dL, (0, p["top"]), (1, 0))
    TLo = tp
    TLi = inter(Li, dL, tp, tdir)
    TRi = inter(Ri, dR, tp, tdir)
    TRo = inter(V0, dR, tp, tdir)
    sil = [TLo, TLi, C, TRi, TRo, V0]
    if not slash:
        return [tidy(sil)]
    c1 = inter(V0, dL, (0, p["cut"] + p["gap"]), (1, 0))  # borde inferior del tajo
    c2 = inter(V0, dL, (0, p["cut"]), (1, 0))             # borde superior
    low = keep(sil, c1, (c1[0] + s[0], c1[1] + s[1]), (V0[0], V0[1] + 10))
    c2b = (c2[0] + s[0], c2[1] + s[1])
    if C[1] < c2[1] + p["slope"] * (C[0] - c2[0]):
        # El tajo pasa bajo la entrepierna: la parte de arriba es una sola pieza.
        ups = [keep(sil, c2, c2b, (TLo[0], TLo[1] - 10))]
    else:
        ups = [keep([TLo, TLi, C, V0], c2, c2b, (TLo[0], TLo[1] - 10)),
               keep([C, TRi, TRo, V0], c2, c2b, (TRo[0], TRo[1] - 10))]
    return [tidy(x) for x in ups + [low] if len(tidy(x)) >= 3]


def bbox(polys):
    xs = [x for p in polys for x, _ in p]
    ys = [y for p in polys for _, y in p]
    return min(xs), min(ys), max(xs), max(ys)


def place(polys, box, margin, optical_dy=0.0):
    """Escala y centra polys dentro de un cuadrado de lado box con margen."""
    x0, y0, x1, y1 = bbox(polys)
    k = (box - 2 * margin) / max(x1 - x0, y1 - y0)
    ox = (box - (x1 - x0) * k) / 2 - x0 * k
    oy = (box - (y1 - y0) * k) / 2 - y0 * k + optical_dy
    return [[(x * k + ox, y * k + oy) for x, y in p] for p in polys]


def tajo_pixel(size, inner):
    """Geometría reconstruida en píxeles enteros para tamaños chicos.

    inner: alto disponible (px) para la V. El vértice y el remate izquierdo caen en
    bordes de píxel, los anchos son enteros y el tajo mide al menos 1 px de alto
    más el antialias de la diagonal.
    """
    p = dict(TAJO)
    k = inner / (TAJO["vy"] - TAJO["top"] + 4.0)  # +4: el remate derecho sube más
    q = lambda v: float(round(v))
    p["top"] = 0.0
    p["vy"] = q((TAJO["vy"] - TAJO["top"]) * k)
    p["vx"] = q(TAJO["vx"] * k)
    p["dL"] = TAJO["dL"]
    p["dR"] = TAJO["dR"]
    p["wL"] = max(2.0, q(TAJO["wL"] * k))
    p["wR"] = max(2.0, q(TAJO["wR"] * k))
    p["cut"] = q((TAJO["cut"] - TAJO["top"]) * k)
    p["gap"] = max(1.25, (TAJO["gap"] * k))
    return tajo(p)


# ---------------------------------------------------------------- raster
def raster(polys, size, color, ss=16, bg=None):
    S = size * ss
    mask = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(mask)
    for p in polys:
        d.polygon([(x * ss, y * ss) for x, y in p], fill=255)
    mask = mask.resize((size, size), Image.BOX)
    img = Image.new("RGBA", (size, size), bg or (0, 0, 0, 0))
    img.paste(Image.new("RGBA", (size, size), color + (255,)), (0, 0), mask)
    return img


def plate_mask(size, ss=16):
    S = size * ss
    m = Image.new("L", (S, S), 0)
    r = round(size * 0.24 * ss)
    inset = 0 if size <= 24 else round(size / 32 * ss)  # 1/32 de aire en los grandes
    ImageDraw.Draw(m).rounded_rectangle((inset, inset, S - 1 - inset, S - 1 - inset), r, fill=255)
    return m.resize((size, size), Image.BOX)


def app_icon(size):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    img.paste(Image.new("RGBA", (size, size), PLATE + (255,)), (0, 0), plate_mask(size))
    if size <= 32:
        inner = {16: 12, 20: 14, 24: 17, 32: 22}[size]
        polys = tajo_pixel(size, inner)
        x0, y0, x1, y1 = bbox(polys)
        dx = round((size - (x1 - x0)) / 2 - x0)
        dy = round((size - (y1 - y0)) / 2 - y0)
        polys = [[(x + dx, y + dy) for x, y in p] for p in polys]
    else:
        polys = place(tajo(), size, size * 0.19)
    mark = raster(polys, size, MINT)
    img.alpha_composite(mark)
    return img


ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 256)


def write_ico(path, images):
    """ICO con DIB 32 bpp (BGRA + máscara AND) en los chicos y PNG en 256."""
    entries = []
    for im in images:
        w, h = im.size
        if w >= 256:
            buf = io.BytesIO()
            im.save(buf, "PNG", optimize=True)
            data = buf.getvalue()
        else:
            px = im.convert("RGBA").tobytes("raw", "BGRA")
            rows = [px[y * w * 4:(y + 1) * w * 4] for y in range(h)][::-1]
            xor = b"".join(rows)
            mask_row = ((w + 31) // 32) * 4
            alpha = im.getchannel("A").tobytes()
            andm = bytearray()
            for y in range(h - 1, -1, -1):
                row = bytearray(mask_row)
                for x in range(w):
                    if alpha[y * w + x] == 0:
                        row[x // 8] |= 0x80 >> (x % 8)
                andm += row
            hdr = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(xor) + len(andm), 0, 0, 0, 0)
            data = hdr + xor + bytes(andm)
        entries.append((w, h, data))
    out = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    body = b""
    for w, h, data in entries:
        out += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        body += data
    with open(path, "wb") as f:
        f.write(out + body)


# ---------------------------------------------------------------- SVG
def svg_path(polys, nd=2):
    def f(v):
        s = f"{v:.{nd}f}".rstrip("0").rstrip(".")
        return s if s not in ("-0", "") else "0"

    parts = []
    for p in polys:
        parts.append("M" + " ".join(f"{f(x)} {f(y)}" for x, y in p) + "Z")
    return "".join(parts)


def mark_svg(color_hex):
    polys = place(tajo(), 64, 4)
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">'
        f'<path fill="{color_hex}" d="{svg_path(polys)}"/></svg>\n'
    )


def wordmark_svg(color_hex):
    try:
        from fontTools.pens.svgPathPen import SVGPathPen
        from fontTools.pens.transformPen import TransformPen
        from fontTools.ttLib import TTFont
        from fontTools.varLib import instancer
    except ImportError:
        print("fontTools no disponible: se omite el wordmark", file=sys.stderr)
        return None
    font = TTFont(os.path.join(ROOT, "qml", "fonts", "Figtree-Variable.ttf"))
    font = instancer.instantiateVariableFont(font, {"wght": 700})
    gs = font.getGlyphSet()
    cmap = font.getBestCmap()
    upm = font["head"].unitsPerEm
    cap = font["OS/2"].sCapHeight
    H = 64.0                     # alto de la marca
    scale = (H * 0.62) / cap     # la altura de mayúscula = 62 % de la marca
    base = 32 + cap * scale / 2  # mayúsculas centradas ópticamente con la V
    mark = place(tajo(), 64, 4)
    x = 64 + 10.0
    pen = SVGPathPen(gs, ntos=lambda v: (f"{v:.2f}".rstrip("0").rstrip(".") or "0"))
    tracking = -0.01 * upm
    for ch in "Velo":
        g = cmap[ord(ch)]
        tp = TransformPen(pen, (scale, 0, 0, -scale, x, base))
        gs[g].draw(tp)
        x += (gs[g].width + tracking) * scale
    width = x + 2
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="64" viewBox="0 0 {width:.2f} 64">'
        f'<path fill="{color_hex}" d="{svg_path(mark)}{pen.getCommands()}"/></svg>\n'
    )


def main():
    icons = os.path.join(ROOT, "qml", "icons")
    with open(os.path.join(icons, "velo.svg"), "w", encoding="utf8", newline="\n") as f:
        f.write(mark_svg(GREEN_HEX))
    with open(os.path.join(icons, "velo_mono.svg"), "w", encoding="utf8", newline="\n") as f:
        f.write(mark_svg("#000000"))
    wm = wordmark_svg(GREEN_HEX)
    if wm:
        with open(os.path.join(icons, "velo_wordmark.svg"), "w", encoding="utf8", newline="\n") as f:
            f.write(wm)

    assets = os.path.join(ROOT, "crates", "app", "assets")
    os.makedirs(assets, exist_ok=True)
    write_ico(os.path.join(assets, "velo.ico"), [app_icon(s) for s in ICO_SIZES])


if __name__ == "__main__":
    main()
