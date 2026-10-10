.pragma library

// Geometry for the visualizer (task B4). They run once per drawn band set: each published
// `bands` array (only while Visualizer.running, at the screen's rate) and, when the bands arrive
// slower than the screen refreshes, the few in-between frames of a glide (G4). Never while idle.
// R2: the per-frame variants (*Into) write into buffers the caller owns and reuses (arrays and
// their Qt.point objects are kept and overwritten), so drawing a frame allocates nothing in JS
// and never feeds the garbage collector. `gain` (the volume, V1b) is applied while reading the
// bands instead of in a copy. The plain variants (wave, ring, mix...) allocate and stay for tests.

// Scratch rows for soften(): temporary results, valid until the next call (one thread).
var scratchA = []
var scratchB = []

function equal(a, b) {
    if (!a || !b || a.length !== b.length)
        return false
    for (let i = 0; i < a.length; ++i)
        if (a[i] !== b[i]) return false
    return true
}

// Writes point k of `out` (reusing the object already there).
function put(out, k, x, y) {
    if (k < out.length) {
        const p = out[k]
        p.x = x
        p.y = y
    } else {
        out.push(Qt.point(x, y))
    }
}

// Copies the levels of `src` into `dst` and says whether they were already equal (then the
// drawing built from them does not need to be rebuilt).
function sameInto(dst, src) {
    const n = src ? src.length : 0
    let same = dst.length === n
    for (let i = 0; i < n; ++i) {
        if (same && dst[i] !== src[i])
            same = false
        dst[i] = src[i]
    }
    dst.length = n
    return same
}

// Rust applies the soft knee and reserves headroom. Do not clamp the crests
// again here: clipping neighbour values before interpolation creates plateaus.
function level(v) {
    return isFinite(v) && v > 0 ? v : 0
}

// Value of the band curve at fractional position p in [0, n-1], cosine-eased between neighbours
// so the outline is smooth without needing curve path elements.
function sample(bands, p) {
    const n = bands.length
    if (n === 0)
        return 0
    const i = Math.min(n - 1, Math.max(0, Math.floor(p)))
    const j = Math.min(n - 1, i + 1)
    const t = (1 - Math.cos((p - i) * Math.PI)) / 2
    return level(bands[i]) * (1 - t) + level(bands[j]) * t
}

// Spreads each band into its neighbours with `passes` rounds of a 1-2-1 kernel (two passes ~ a
// 1-4-6-4-1 Gaussian). Every band is normalised against its own history, so adjacent values can
// differ a lot; without this the wave reads as spikes instead of hills (user, 2026-10-08).
// R2: returns a scratch row (do not keep it); `gain` scales every level (default 1).
function soften(bands, passes, gain) {
    const g = gain === undefined ? 1 : gain
    const n = bands.length
    let cur = scratchA, next = scratchB
    cur.length = n
    for (let i = 0; i < n; ++i)
        cur[i] = level(bands[i]) * g
    for (let p = 0; p < passes && n > 2; ++p) {
        next.length = n
        for (let i = 0; i < n; ++i)
            next[i] = (cur[Math.max(0, i - 1)] + 2 * cur[i] + cur[Math.min(n - 1, i + 1)]) / 4
        const t = cur
        cur = next
        next = t
    }
    return cur
}

// Catmull-Rom through the band values at fractional position p in [0, n-1]: the slope carries
// across each band, so crests are rounded instead of the flat tops of cosine easing.
function spline(v, p) {
    const n = v.length
    const i = Math.min(n - 1, Math.max(0, Math.floor(p)))
    const t = p - i
    const p0 = v[Math.max(0, i - 1)], p1 = v[i]
    const p2 = v[Math.min(n - 1, i + 1)], p3 = v[Math.min(n - 1, i + 2)]
    return 0.5 * (2 * p1 + (p2 - p0) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t
                  + (3 * p1 - p0 - 3 * p2 + p3) * t * t * t)
}

// G4: soft ceiling for a drawn level. Identity up to `knee`; above it the value bends toward 1
// (tanh), so crests that the spline or a loud frame push past 1 stay inside the box without the
// flat plateau of a hard clamp. Applied after interpolation, never to the raw bands.
function limit(y, knee) {
    if (!(knee > 0 && knee < 1) || y <= knee)
        return y
    const room = 1 - knee
    const e = Math.exp(-2 * (y - knee) / room)
    return knee + room * (1 - e) / (1 + e)
}

// G4: element-wise blend of two band sets (a glide between two published frames). Missing or
// differently sized input falls back to `b`.
function mix(a, b, t) {
    if (t >= 1 || !a || a.length !== b.length)
        return b
    return mixInto([], a, b, t)
}
// R2: the same blend written into `out` (reused every glide frame).
function mixInto(out, a, b, t) {
    const n = b.length
    out.length = n
    for (let i = 0; i < n; ++i)
        out[i] = level(a[i]) + (level(b[i]) - level(a[i])) * t
    return out
}

// Closed polygon for the horizontal wave: the top edge follows the softened bands (left to
// right), then the bottom corners close it. `floor` keeps a hairline visible at silence.
// G4: `inset` (px) keeps the crest that far below the top edge (room for an outline stroke) and
// `knee` softly limits the level, so the polygon never leaves its w × h box.
function wave(bands, w, h, sub, floor, passes, inset, knee) {
    return waveInto([], bands, w, h, sub, floor, passes, inset, knee, 1)
}
// R2: the wave polygon written into `out` (its points are reused).
function waveInto(out, bands, w, h, sub, floor, passes, inset, knee, gain) {
    const n = bands ? bands.length : 0
    if (n === 0 || w <= 0 || h <= 0) {
        out.length = 0
        return out
    }
    const top = Math.min(h, Math.max(0, inset || 0))
    const span = h - top
    const v = soften(bands, passes, gain)
    const step = w / n
    const steps = (n - 1) * sub
    let k = 0
    put(out, k++, 0, h - Math.min(1, limit(Math.max(floor, v[0]), knee)) * span)
    for (let j = 0; j <= steps; ++j)
        put(out, k++, step / 2 + j / sub * step, h - Math.min(1, limit(Math.max(floor, spline(v, j / sub)), knee)) * span)
    put(out, k++, w, h - Math.min(1, limit(Math.max(floor, v[n - 1]), knee)) * span)
    put(out, k++, w, h)
    put(out, k++, 0, h)
    out.length = k
    return out
}

// Index into `bands` for bar j of 2n bars laid around a circle, mirrored left/right so the
// seam at the top and bottom matches (bass at the top, treble at the bottom).
function mirrorIndex(j, n) {
    return j < n ? j : 2 * n - 1 - j
}

// Closed blob around the cover: radius = inner + band * (outer - inner), `m` samples per turn,
// starting at 12 o'clock and going clockwise, mirrored like the bars.
function ring(bands, cx, cy, inner, outer, m, floor) {
    return ringInto([], bands, cx, cy, inner, outer, m, floor, 1)
}
// R2: the blob written into `out` (its points are reused); `gain` scales the levels.
function ringInto(out, bands, cx, cy, inner, outer, m, floor, gain) {
    const n = bands ? bands.length : 0
    const g = gain === undefined ? 1 : gain
    if (n === 0 || outer <= inner) {
        out.length = 0
        return out
    }
    for (let j = 0; j <= m; ++j) {
        const u = (j % m) / m * 2                 // 0..2
        const p = (u <= 1 ? u : 2 - u) * (n - 1)  // 0..n-1..0
        // Light smoothing (1-2-1 over +-0.7 band) so the blob reads as one soft shape.
        const lo = sample(bands, Math.max(0, p - 0.7)), hi = sample(bands, Math.min(n - 1, p + 0.7))
        const v = Math.max(floor, g * (lo + 2 * sample(bands, p) + hi) / 4)
        const r = inner + v * (outer - inner)
        const t = j / m * 2 * Math.PI
        put(out, j, cx + r * Math.sin(t), cy - r * Math.cos(t))
    }
    out.length = m + 1
    return out
}

// S9/N1: top edge only of the horizontal wave (same points as wave(), without the closing
// corners), for the outline stroke of the player-bar curve.
function crest(bands, w, h, sub, floor, passes, inset, knee) {
    return crestOf(wave(bands, w, h, sub, floor, passes, inset, knee))
}
// G4: the crest of an already built wave polygon (so fill and outline share one computation).
function crestOf(pts) {
    return pts.length > 3 ? pts.slice(0, pts.length - 2) : []
}
// R2: the crest copied into `out` (its points are reused).
function crestInto(out, pts) {
    const n = pts.length > 3 ? pts.length - 2 : 0
    for (let k = 0; k < n; ++k)
        put(out, k, pts[k].x, pts[k].y)
    out.length = n
    return out
}

// S9/N1: `count` thin vertical bars across `w` (the player-bar spectrum), as segments for one
// PathMultiline (a single Shape, no Item per bar). The bands are softened once and resampled
// with the Catmull-Rom spline, so 150+ bars read as one smooth spectrum. `inset` keeps the
// round caps of a stroke of that half-width inside the item.
// G4: the bar tips stay inside [inset, h - inset] (round caps included) and loud bands bend
// softly toward the top (`knee`) instead of all hitting the same flat ceiling.
function spectrum(bands, w, h, count, floor, passes, inset, knee) {
    return spectrumInto([], bands, w, h, count, floor, passes, inset, knee, 1)
}
// R2: the segments written into `out` (each segment's array and points are reused).
function spectrumInto(out, bands, w, h, count, floor, passes, inset, knee, gain) {
    const n = bands ? bands.length : 0
    if (n === 0 || w <= 0 || h <= 0 || count <= 0) {
        out.length = 0
        return out
    }
    const v = soften(bands, passes, gain)
    const step = w / count
    const base = h - inset
    const span = Math.max(0, h - 2 * inset)
    for (let k = 0; k < count; ++k) {
        const p = n > 1 ? (k + 0.5) / count * (n - 1) : 0
        const y = Math.min(1, limit(Math.max(floor, n > 1 ? spline(v, p) : v[0]), knee))
        const x = step * (k + 0.5)
        if (k >= out.length)
            out.push([])
        const seg = out[k]
        put(seg, 0, x, base)
        put(seg, 1, x, base - y * span)
        seg.length = 2
    }
    out.length = count
    return out
}
