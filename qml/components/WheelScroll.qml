import QtQuick
import "../theme"

// Mouse wheel for a vertical Flickable / ListView / GridView (lavado de cara, Ajustes).
// Qt's own wheel step is ~20 px per notch, which feels glacial on a tall page. This handler
// moves `rowsPerNotch` rows per notch (a row = `rowHeight` px), capped at `maxWindowShare` of the
// viewport so a short window never jumps past a screenful, and glides there (contentY only,
// `Theme.motion.base`, ease-out). Interruptible: the next notch retargets from where the glide
// is (accumulating), a drag/touch/scrollbar takes over at once, and `stop()` yields to any other
// programmatic scroll (e.g. an index glide). With motion off (Modo rendimiento / Reducir
// movimiento) the page jumps straight to the target.
//
// Use: declare it INSIDE the Flickable (its parent is the scope), nothing else needed:
//     Flickable { id: flick; ... WheelScroll { rowHeight: Theme.size.trackRow } }
// Touchpads: a non-zero `pixelDelta` is applied 1:1 with no glide. Ctrl/Shift/Alt + wheel is not
// ours (`acceptedModifiers: NoModifier`), so a zoom shortcut or horizontal scroll still gets it.
WheelHandler {
    id: wheel

    // The scrolling view; defaults to the Flickable this handler is declared in (Qt parents
    // handlers declared inside a Flickable to its contentItem, so one level up is looked at).
    property Item flickable: parent && parent.contentHeight !== undefined ? parent : (parent ? parent.parent : null)
    property real rowHeight: Theme.size.trackRow     // px of one list row / form row
    property real rowsPerNotch: 3
    property real maxWindowShare: 0.4                // one notch never exceeds this share of the view height
    readonly property real notchPx: Math.max(Theme.px(20),
        Math.min(rowHeight * rowsPerNotch, flickable.height * maxWindowShare))
    readonly property bool gliding: glide.running

    target: null
    acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
    acceptedModifiers: Qt.NoModifier
    orientation: Qt.Vertical

    function minY() { return flickable.originY - flickable.topMargin }
    function maxY() {
        return Math.max(minY(), flickable.originY + flickable.contentHeight - flickable.height + flickable.bottomMargin)
    }
    function clampY(y) { return Math.max(minY(), Math.min(maxY(), y)) }
    // Stops the glide where it is. Call it before scrolling the view by other means.
    function stop() { glide.stop() }

    readonly property NumberAnimation glide: NumberAnimation {
        target: wheel.flickable
        property: "contentY"
        duration: Math.max(1, Theme.motion.base)
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeOut
    }

    onWheel: event => {
        const v = flickable
        if (!v || (event.angleDelta.y === 0 && event.pixelDelta.y === 0)) { event.accepted = false; return }
        if (v.cancelFlick) v.cancelFlick()
        if (event.pixelDelta.y !== 0) {
            glide.stop()
            v.contentY = clampY(v.contentY - event.pixelDelta.y)
        } else {
            const base = glide.running ? glide.to : v.contentY
            const to = clampY(base - event.angleDelta.y / 120 * notchPx)
            glide.stop()
            if (Theme.motion.scale <= 0 || Math.abs(to - v.contentY) < 1) v.contentY = to
            else { glide.to = to; glide.start() }
        }
        event.accepted = true
    }
}
