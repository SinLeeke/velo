.pragma library

// Scalloped "cookie" outline (Theme.scallop) as a closed polyline for PathPolyline.
// Called once per size change, never per frame.
//   r(t) = R - depth*R*(1 - |cos(lobes*t/2)|)
// gives rounded lobes and small notches between them, like the play disc in the references.
function points(w, h, lobes, depth) {
    const cx = w / 2, cy = h / 2, R = Math.min(w, h) / 2
    const n = lobes * 24
    const pts = []
    for (let i = 0; i <= n; ++i) {
        const t = i / n * 2 * Math.PI
        const r = R - depth * R * (1 - Math.abs(Math.cos(lobes * t / 2)))
        pts.push(Qt.point(cx + r * Math.sin(t), cy - r * Math.cos(t)))
    }
    return pts
}
