import QtQuick
import Velo.Backend
import "../../theme"
import "morph.js" as Morph

// S1-NP: Library ↔ Now Playing choreography (the user's "animación muy exagerada y linda").
// Main.qml hosts the two views in two Loaders: the library (`library`) stays alive under Now
// Playing (`nowPlaying`, built only while it is shown), so the library is always there to come
// back to: no photos, nothing to wait for and never a frame with only the window colour.
// Main hands the view swap to `go(toNowPlaying, apply)`; NowPlaying.qml reads `presence`
// (0 = folded into the library's mini player, 1 = at rest).
//
// Entrance (Theme.motion.npEnter, ~950 ms):
//   1. Now Playing is built at presence 0 over the library: only its cover shows, exactly over
//      the mini player's cover tile (waits up to npArtWait for the big image to decode, so it
//      never flies empty);
//   2. presence 0 → 1 (one linear NumberAnimation; each element eases its own window): the
//      library sinks 1 → npLibrarySink while the blurred room blooms over it, the cover flies out
//      of the tile on an arc and lands with a small pop, the left column and the lyric lines rise
//      in turn, the visualizer grows from its baseline (or the ring opens), the top buttons drop
//      in. Then the library is hidden (visible: false: not painted, no animation left).
// Exit (npExit, ~520 ms): presence 1 → 0 on the live view, the same windows backwards: lyrics
//   and controls sink, the room fades off the library (shown again, rising back to 1 under it),
//   the cover flies back into the tile. At presence 0 Now Playing is destroyed: its cover sits on
//   the library's own tile, so the swap does not show.
// Interruption: a change of mind at any point retargets `presence` from where it is (same
// windows: no jump, nothing is stacked).
// Modo rendimiento / Reducir movimiento: instant. Minimized, hidden or (with "Pausar efectos en
// segundo plano") unfocused mid-way: it settles at once. Paused playback also settles at once.
// Cost: one GUI-thread NumberAnimation and a handful of opacity/transform bindings while it
// runs; no layers, no shaders, no textures, no Timer except the one-shot art wait.
// At rest the library under Now Playing is `visible: false` (not painted) and has nothing that
// ticks by itself (no Timer, no looping animation); a one-shot it may start on a backend change
// (the mini player's cover crossfade) ends on its own. No tree walking: it would stall the UI.
Item {
    id: root

    property Item library: null          // Main's library Loader (alive under Now Playing)
    property Item nowPlaying: null       // Main's Now Playing Loader

    property real presence: 1            // master progress, mirrored into NowPlaying.presence
    property string phase: "idle"        // idle | awaitArt | enter | exit
    property bool wantNowPlaying: false
    property var apply: null             // Main's view swap (idempotent: shownView = wantedView)

    // Mini player cover (loader coordinates), measured at each change; kept relative to the
    // bottom edge so a resize mid-way still lands on it.
    property real tileX: Theme.space.lg
    property real tileBottom: (Theme.size.playerBar + Theme.size.trackRow) / 2
    property real tileSize: Theme.size.trackRow
    readonly property rect tileRect: Qt.rect(tileX, height - tileBottom, tileSize, tileSize)

    readonly property bool busy: phase !== "idle"
    readonly property bool motionStopped: !Player.isPlaying || Theme.motion.scale === 0 || !windowShown()
        || (Theme.pauseEffectsInBackground && !root.Window.active)
    onMotionStoppedChanged: if (motionStopped && busy) settleNow()
    // The Now Playing view being driven.
    readonly property Item view: nowPlaying && nowPlaying.item && nowPlaying.item.objectName === "nowPlayingView"
                                 ? nowPlaying.item : null
    readonly property bool drivesView: busy && view !== null

    visible: false                       // draws nothing itself

    // ------------------------------------------------------------ API for Main.qml
    // True when this change belongs to the choreography (call go() then).
    function takes(shown, wanted) {
        const views = ["library", "nowplaying"]
        if (views.indexOf(wanted) < 0) {
            cancel()
            return false
        }
        if (busy)
            return true
        return shown !== wanted && views.indexOf(shown) >= 0
    }
    function go(toNowPlaying, swap) {
        wantNowPlaying = toNowPlaying
        apply = swap
        if (motionStopped) {
            settleNow()
            return
        }
        switch (phase) {
        case "idle":
            if (toNowPlaying) beginEnter(); else runExit()
            break
        case "awaitArt":
            if (!toNowPlaying) runExit()     // still folded: the exit ends at once
            break
        default:                             // enter / exit: retarget from where it is
            if (toNowPlaying) runEnter(); else runExit()
            break
        }
    }
    // Another view (login) takes over: drop everything without applying anything.
    function cancel() {
        if (busy)
            reset()
    }

    // Preview only: hold the choreography at a point.
    function freeze(p) { run.stop(); artWait.stop(); presence = p }

    // ------------------------------------------------------------ steps
    function windowShown() {
        const w = root.Window.window
        return w !== null && w.visible && w.visibility !== Window.Minimized && w.visibility !== Window.Hidden
    }
    function settleNow() {
        const fn = apply
        if (!wantNowPlaying && view)
            view.prepareUnload()
        // Release Bindings before the exit callback destroys their target.
        reset()
        if (fn)
            fn()
    }
    function reset() {
        run.stop()
        artWait.stop()
        phase = "idle"                   // the Bindings let go: presence back to 1, library at 1
        presence = 1
        apply = null
    }

    function measureTile() {
        const lib = library ? library.item : null
        const kids = lib ? lib.children : []
        for (let i = 0; i < kids.length; ++i) {
            const bar = kids[i]
            if (bar && bar.objectName === "libraryPlayerBar" && bar.coverTile && bar.visible) {
                const r = bar.coverTile.mapToItem(lib, 0, 0, bar.coverTile.width, bar.coverTile.height)
                tileX = r.x
                tileBottom = lib.height - r.y
                tileSize = r.width
                return
            }
        }
        tileX = Theme.space.lg
        tileBottom = (Theme.size.playerBar + Theme.size.trackRow) / 2
        tileSize = Theme.size.trackRow
    }

    function beginEnter() {
        measureTile()
        presence = 0
        phase = "awaitArt"
        if (apply)
            apply()                      // Now Playing is built now, already at presence 0
        if (!view) {                     // nothing to drive
            settleNow()
            return
        }
        if (view.coverReady === false)
            artWait.start()
        else
            runEnter()
    }
    function runEnter() {
        if (motionStopped) { settleNow(); return }
        artWait.stop()
        phase = "enter"
        run.stop()
        run.to = 1
        run.duration = Math.max(1, Theme.motion.npEnter * (1 - presence))
        run.start()
    }
    function runExit() {
        if (motionStopped) { settleNow(); return }
        if (phase === "idle")
            measureTile()                // the library kept its layout while hidden
        artWait.stop()
        phase = "exit"
        run.stop()
        run.to = 0
        run.duration = Math.max(1, Theme.motion.npExit * presence)
        run.start()
    }
    function runDone() {
        // Exit: Now Playing goes (its cover is on the library's own tile now). Entrance: the
        // view is at rest and the library hides under it.
        settleNow()
    }

    NumberAnimation {
        id: run
        target: root
        property: "presence"
        // Linear on purpose: every element eases its own window.
        easing.type: Easing.Linear
        onFinished: root.runDone()
    }
    // One shot, only while an entrance waits for the big cover to decode.
    Timer {
        id: artWait
        interval: Theme.motion.npArtWait
        onTriggered: if (root.phase === "awaitArt") root.runEnter()
    }
    Connections {
        target: root.phase === "awaitArt" ? root.view : null
        ignoreUnknownSignals: true
        function onCoverReadyChanged() {
            if (root.view.coverReady)
                root.runEnter()
        }
    }
    // Minimized (or hidden) mid-way: settle now, so nothing keeps animating unseen.
    Connections {
        target: root.busy ? root.Window.window : null
        ignoreUnknownSignals: true
        function onVisibilityChanged() {
            if (!root.windowShown())
                root.settleNow()
        }
    }

    // ------------------------------------------------------------ the views
    Binding {
        target: root.view
        property: "presence"
        value: root.presence
        when: root.drivesView
        restoreMode: Binding.RestoreBindingOrValue
    }
    Binding {
        target: root.view
        property: "morphFrom"
        value: root.tileRect
        when: root.drivesView
        restoreMode: Binding.RestoreBindingOrValue
    }
    // The library recedes under the blooming room (and rises back on the way out). Fully
    // opaque the whole time: the room fades over it, so the window colour never shows through.
    Binding {
        target: root.library
        property: "scale"
        value: Morph.mix(1, Theme.motion.npLibrarySink, Morph.step(root.presence, Theme.motion.npWinBackdrop, Theme.motion.npEase))
        when: root.busy
        restoreMode: Binding.RestoreBindingOrValue
    }
}
