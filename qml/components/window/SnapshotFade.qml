import QtQuick
import "../../theme"

// One-shot window snapshot crossfade (S1: A1 light/dark, B1 Library ↔ Now Playing ↔ Login).
//
//   run(apply, duration, curve, zoomTarget)
//     1. photographs `source` (grabToImage: one texture, rendered once),
//     2. shows that photo on top of everything and calls `apply()` underneath (theme flip,
//        view swap: all of it happens at once, hidden by the photo),
//     3. fades the photo 1 → 0 with an OpacityAnimator (render thread, no QML work per frame);
//        `zoomTarget`, when given, settles 0.98 → 1 with a ScaleAnimator over the same time,
//     4. drops the photo and its texture.
//
// Interruption: a new run() while one is fading stops the fade where it is (the Animator writes
// its current value back), photographs what is on screen right now (the half-faded photo
// included) and starts again from there, so nothing jumps. While a grab is in
// flight, the latest `apply` wins. When the window is hidden/minimized or `duration` is 0
// (Modo rendimiento), `apply()` runs at once with no grab.
// The photo takes no input: Image accepts no mouse buttons, so clicks and the native title-bar
// hit test (window_chrome.cpp) see straight through it.
Item {
    id: root

    // What to photograph: the window's content item (this component must be inside it).
    property Item source: null

    property var pendingApply: null
    property int pendingDuration: 0
    property var pendingCurve: Theme.motion.easeInOut
    property Item pendingZoom: null
    property bool grabbing: false
    property bool halting: false        // Animators emit finished() on stop(): ignore it then
    property var grabResult: null       // keeps the snapshot texture alive while it is shown

    visible: false
    Connections {
        target: Theme
        function onEffectsAllowedChanged() {
            if (Theme.effectsAllowed) return
            const apply = root.pendingApply
            root.pendingApply = null
            root.finish()
            if (apply) apply()
        }
    }

    function windowShown() {
        const w = root.Window.window
        return Theme.effectsAllowed && w !== null && w.visible && w.visibility !== Window.Minimized && w.visibility !== Window.Hidden
    }

    function run(apply, duration, curve, zoomTarget) {
        if (duration <= 0 || !source || !windowShown()) {
            pendingApply = null              // a grab in flight has nothing left to apply
            finish()
            apply()
            return
        }
        pendingApply = apply
        pendingDuration = duration
        pendingCurve = curve
        pendingZoom = zoomTarget || null
        if (grabbing)
            return                       // the grab in flight applies the latest request
        // Animators only write their value back to the item when stopped: stop them first so
        // the grab sees the half-faded photo / half-settled view that is on screen right now.
        halt()
        grabbing = true
        const started = source.grabToImage(function (result) {
            root.grabbing = false
            root.show(result)
        })
        if (!started) {
            grabbing = false
            const fn = pendingApply
            pendingApply = null
            finish()
            fn()
        }
    }

    function show(result) {
        const apply = pendingApply
        pendingApply = null
        if (!apply)
            return
        halt()
        if (zoom.target)
            zoom.target.scale = 1
        // The window colour sits under the photo: grabToImage leaves out the window background.
        const w = root.Window.window
        backdrop.color = w ? w.color : Theme.library.clearPaint
        grabResult = result
        photo.source = result.url
        root.opacity = 1
        root.visible = true
        apply()
        fade.duration = pendingDuration
        fade.easing.bezierCurve = pendingCurve
        fade.start()
        if (pendingZoom && Theme.motion.scale > 0) {
            // The incoming view settles 0.98 → 1 underneath the fading photo (never from 0).
            zoom.target = pendingZoom
            zoom.duration = pendingDuration
            zoom.start()
        }
    }

    function halt() {
        halting = true
        fade.stop()
        zoom.stop()
        halting = false
    }

    function finish() {
        halt()
        if (zoom.target)
            zoom.target.scale = 1
        root.visible = false
        root.opacity = 1
        photo.source = ""
        grabResult = null
    }

    Rectangle {
        id: backdrop
        anchors.fill: parent
    }
    Image {
        id: photo
        anchors.fill: parent
        asynchronous: false              // the grab is already in memory: show it this frame
        cache: false
        smooth: false                    // 1:1 with the window, no filtering needed
    }

    OpacityAnimator {
        id: fade
        target: root
        from: 1
        to: 0
        easing.type: Easing.Bezier
        onFinished: if (!root.halting) root.finish()
    }
    ScaleAnimator {
        id: zoom
        from: Theme.motion.viewZoomFrom
        to: 1
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeOut
    }
}
