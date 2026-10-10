.pragma library

// S1-NP: pure helpers for the Now Playing choreography (no state, no timers).
// Every value is a function of the master progress `presence` (0..1), so the exit can play the
// same windows backwards and a change of mind halfway never jumps.

function clamp01(v) {
    return v < 0 ? 0 : v > 1 ? 1 : v
}

// Local progress (0..1) of a window [start, end] of the master progress.
function seg(p, win) {
    return clamp01((p - win[0]) / (win[1] - win[0]))
}

function bez(u, a, b) {
    return ((1 - 3 * b + 3 * a) * u + (3 * b - 6 * a)) * u * u + 3 * a * u
}
function dbez(u, a, b) {
    return (3 * (1 - 3 * b + 3 * a) * u + 2 * (3 * b - 6 * a)) * u + 3 * a
}

// Cubic bezier easing with CSS cubic-bezier semantics; `curve` is a Theme.motion curve
// ([x1, y1, x2, y2, 1, 1], QML's bezierCurve form). y may leave 0..1 (overshoot).
function ease(curve, t) {
    if (t <= 0)
        return 0
    if (t >= 1)
        return 1
    const x1 = curve[0], y1 = curve[1], x2 = curve[2], y2 = curve[3]
    let u = t
    for (let i = 0; i < 8; ++i) {          // Newton on x(u) = t
        const dx = bez(u, x1, x2) - t
        if (Math.abs(dx) < 1e-5)
            return bez(u, y1, y2)
        const d = dbez(u, x1, x2)
        if (Math.abs(d) < 1e-6)
            break
        u = clamp01(u - dx / d)
    }
    let lo = 0, hi = 1
    u = t
    for (let i = 0; i < 24; ++i) {         // bisection fallback
        const x = bez(u, x1, x2)
        if (Math.abs(x - t) < 1e-5)
            break
        if (x < t)
            lo = u
        else
            hi = u
        u = (lo + hi) / 2
    }
    return bez(u, y1, y2)
}

// Eased local progress of a window.
function step(p, win, curve) {
    return ease(curve, seg(p, win))
}

function mix(a, b, t) {
    return a + (b - a) * t
}

// Landing pop: 0 → 1 → 0 over t (0..1). It leaves 0 with speed (the impact), peaks near a third
// and settles softly (zero speed at 1), so a value carried by it rests exactly where it started.
function pop(t) {
    if (t <= 0 || t >= 1)
        return 0
    return Math.sin(Math.PI * (1 - Math.pow(1 - t, 1.6)))
}

// Row-major 4×4 (for Qt.matrix4x4) of a card tilted by `ax` around the x axis and `ay` around
// the y axis (degrees), about its centre (cx, cy), seen from `d` px. Positive z faces the
// viewer: the side with negative z shrinks with perspective, i.e. sinks back.
function tilt(cx, cy, ax, ay, d) {
    const a = ax * Math.PI / 180, b = ay * Math.PI / 180
    const ca = Math.cos(a), sa = Math.sin(a), cb = Math.cos(b), sb = Math.sin(b)
    // R = Ry(b) · Rx(a); only the rows that act on the z = 0 plane matter.
    const r00 = cb, r01 = sb * sa, r10 = 0, r11 = ca, r20 = -sb, r21 = cb * sa
    const p0 = -r20 / d, p1 = -r21 / d
    const w0 = 1 - p0 * cx - p1 * cy
    return [
        r00 + cx * p0, r01 + cx * p1, 0, -r00 * cx - r01 * cy + cx * w0,
        r10 + cy * p0, r11 + cy * p1, 0, -r10 * cx - r11 * cy + cy * w0,
        0, 0, 1, 0,
        p0, p1, 0, w0
    ]
}
