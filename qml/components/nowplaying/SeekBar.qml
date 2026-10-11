import QtQuick
import QtQuick.Shapes
import "../../theme"

// Elapsed · track · −remaining. The played part is a wavy line while playing (ref_05) and a
// straight line when paused. The wave polyline is rebuilt only when the width or the progress
// changes (positionMs arrives at <= 4 Hz and not at all in pause). While playing the wave drifts
// (C5): one wavelength more than needed is built once and only its Translate moves, clipped to
// the played part. It is stepped by an existing clock when there is one (`clock`, the visualizer's
// bands at the screen's rate, while `clockRunning`). Without that clock (visualizer off, remote
// device), or with motion off, the played part is deliberately straight. No additional clock.
// Click or drag to seek; Player.seek is called once, on release.
Item {
    id: root


    property int positionMs: 0
    property int durationMs: 0
    property bool playing: false
    signal seekRequested(int ms)

    // Any value that changes on an existing clock (NowPlaying passes Visualizer.bands), and
    // whether that clock is ticking (Visualizer.running).
    property var clock: null
    property bool clockRunning: false
    // Drift of the wave within one wavelength (px), advanced by the real time between ticks.
    property real waveShift: 0
    property double lastTick: 0
    readonly property bool waveMoving: playing && !dragging && visible && clockRunning && onScreen && Theme.motion.scale > 0
    readonly property bool onScreen: Window.window !== null && Window.visibility !== Window.Minimized
                                     && Window.visibility !== Window.Hidden
                                     && (!Theme.pauseEffectsInBackground || Window.active)
    readonly property real waveOffset: waveShift % Theme.nowPlaying.waveLength
    onWaveMovingChanged: lastTick = 0
    onClockChanged: {
        if (!waveMoving || !clockRunning) {
            lastTick = 0
            return
        }
        const now = Date.now()
        const dt = lastTick > 0 ? Math.min(Theme.feel.waveMaxStep, (now - lastTick) / 1000) : 0
        lastTick = now
        waveShift = (waveShift + dt * Theme.feel.waveSpeed) % Theme.nowPlaying.waveLength
    }

    property bool dragging: false
    property real dragFraction: 0
    readonly property real fraction: dragging ? dragFraction
                                   : durationMs > 0 ? Math.max(0, Math.min(1, positionMs / durationMs)) : 0
    readonly property int shownMs: Math.round(fraction * durationMs)

    implicitHeight: Theme.size.iconButton - Theme.space.sm
    // T4: where the track starts (and, mirrored, ends): the time column plus its gap. The volume
    // row under it lines its own track up with this one.
    readonly property real trackInset: track.x

    activeFocusOnTab: true
    // Focus taken with the mouse shows no ring (like Control.visualFocus); Tab shows it.
    property bool mouseFocus: false
    onActiveFocusChanged: if (!activeFocus) mouseFocus = false
    // QAccessibleValueInterface reads these properties from the slider item.
    readonly property int value: shownMs
    readonly property int minimumValue: 0
    readonly property int maximumValue: Math.max(0, durationMs)
    readonly property int stepSize: Theme.sliderInput.seekStepMs

    function requestSeek(ms) {
        if (enabled && durationMs > 0 && !dragging)
            seekRequested(Math.max(minimumValue, Math.min(maximumValue, Math.round(ms))))
    }
    Keys.onPressed: event => {
        if (!enabled || durationMs <= 0 || dragging || event.modifiers !== Qt.NoModifier)
            return
        switch (event.key) {
        case Qt.Key_Right: case Qt.Key_Up: requestSeek(value + stepSize); break
        case Qt.Key_Left: case Qt.Key_Down: requestSeek(value - stepSize); break
        case Qt.Key_PageUp: requestSeek(value + Theme.sliderInput.seekPageMs); break
        case Qt.Key_PageDown: requestSeek(value - Theme.sliderInput.seekPageMs); break
        case Qt.Key_Home: requestSeek(minimumValue); break
        case Qt.Key_End: requestSeek(maximumValue); break
        default: return
        }
        event.accepted = true
    }
    Rectangle {
        anchors.fill: parent
        radius: Theme.radius.xs
        color: Theme.library.clearPaint
        border.width: Theme.stroke.focus
        border.color: Theme.cover.ink
        visible: root.activeFocus && !root.mouseFocus
        // Focus ring: instant on purpose (keyboard feedback must not lag).
    }

    function format(ms) {
        const s = Math.max(0, Math.floor(ms / 1000))
        const h = Math.floor(s / 3600), m = Math.floor(s / 60) % 60, sec = s % 60
        const ss = (sec < 10 ? "0" : "") + sec
        return h > 0 ? h + ":" + (m < 10 ? "0" : "") + m + ":" + ss : m + ":" + ss
    }

    TextMetrics {
        id: timeWidth
        font: elapsed.font
        text: root.durationMs >= 3600000 ? "−0:00:00" : "−00:00"
    }

    Text {
        id: elapsed
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        width: timeWidth.advanceWidth
        text: root.format(root.shownMs)
        color: Theme.coverInkSecondary
        font.family: Theme.family
        font.pixelSize: Theme.type.micro
        font.weight: Theme.type.medium
        font.features: Theme.tabularFeatures
        horizontalAlignment: Text.AlignLeft
    }

    Text {
        id: remaining
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: timeWidth.advanceWidth
        text: "−" + root.format(root.durationMs - root.shownMs)
        color: Theme.coverInkSecondary
        font: elapsed.font
        horizontalAlignment: Text.AlignRight
    }

    Item {
        id: track
        anchors.left: elapsed.right
        anchors.right: remaining.left
        anchors.leftMargin: Theme.space.md
        anchors.rightMargin: Theme.space.md
        anchors.verticalCenter: parent.verticalCenter
        height: 2 * Theme.nowPlaying.waveAmplitude + Theme.stroke.progressWave

        readonly property real headX: Math.round(width * root.fraction)
        readonly property bool wavy: root.waveMoving

        // Unplayed rest of the track.
        Rectangle {
            x: track.headX
            width: track.width - x
            height: Theme.stroke.progress
            radius: height / 2
            anchors.verticalCenter: parent.verticalCenter
            color: Theme.cover.inkFaint
        }

        // A-N2: straight and wavy played parts crossfade (opacity only, `fast`, easeOut) instead
        // of swapping on pause / drag / clock loss. Both stay built while one fades out; the wave
        // polyline is only computed while it is on screen (waveShown), so the idle cost is the same.
        // A-N4: while the bar is locked (DJ Livi speaks, nothing to seek) the track dims.
        opacity: root.enabled ? 1 : Theme.library.disabledOpacity
        Behavior on opacity {
            OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        readonly property bool waveShown: wavy || waveLayer.opacity > 0

        // Played part, paused / dragging: straight.
        Rectangle {
            id: straightPlayed
            visible: !track.wavy || opacity > 0
            opacity: track.wavy ? 0 : 1
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            width: track.headX
            height: Theme.stroke.progress
            radius: height / 2
            anchors.verticalCenter: parent.verticalCenter
            color: Theme.cover.ink
        }

        // Played part, playing: wave. The polyline spans headX + one wavelength and is clipped to
        // [0, headX]; drifting only moves its Translate (no rebuild, no repaint of the geometry).
        // The clip cuts the stroke flat, so two dots redraw the round caps where it is cut.
        Item {
            id: waveLayer
            objectName: "seekWaveLayer"
            anchors.fill: parent
            visible: track.headX > 0 && (track.wavy || opacity > 0)
            opacity: track.wavy ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            Item {
                width: track.headX
                height: track.height
                clip: true
                Shape {
                    width: track.headX + Theme.nowPlaying.waveLength
                    height: track.height
                    preferredRendererType: Shape.CurveRenderer
                    transform: Translate { x: root.waveOffset - Theme.nowPlaying.waveLength }
                    ShapePath {
                        strokeColor: Theme.cover.ink
                        strokeWidth: Theme.stroke.progressWave
                        fillColor: Theme.library.clearPaint
                        capStyle: ShapePath.RoundCap
                        joinStyle: ShapePath.RoundJoin
                        PathPolyline { path: track.wave }
                    }
                }
            }
            Rectangle {   // cap at the start
                width: Theme.stroke.progressWave
                height: width
                radius: width / 2
                color: Theme.cover.ink
                x: -width / 2
                y: track.waveY(0) - height / 2
            }
            Rectangle {   // cap at the head
                width: Theme.stroke.progressWave
                height: width
                radius: width / 2
                color: Theme.cover.ink
                x: track.headX - width / 2
                y: track.waveY(track.headX) - height / 2
            }
        }
        // Height of the drifted wave at x (track coordinates).
        function waveY(x) {
            return height / 2 - Theme.nowPlaying.waveAmplitude
                   * Math.sin(2 * Math.PI / Theme.nowPlaying.waveLength * (x - root.waveOffset))
        }

        // Rebuilt only when headX (width × progress) changes; the drift is the Translate above.
        property var wave: {
            if (!waveShown)
                return []
            const mid = height / 2, end = headX + Theme.nowPlaying.waveLength, step = 2, pts = []
            const k = 2 * Math.PI / Theme.nowPlaying.waveLength
            for (let x = 0; x < end; x += step)
                pts.push(Qt.point(x, mid - Theme.nowPlaying.waveAmplitude * Math.sin(k * x)))
            pts.push(Qt.point(end, mid - Theme.nowPlaying.waveAmplitude * Math.sin(k * end)))
            return pts
        }

        MouseArea {
            anchors.fill: parent
            anchors.topMargin: -Theme.space.sm
            anchors.bottomMargin: -Theme.space.sm
            enabled: root.enabled && root.durationMs > 0
            preventStealing: true
            function fractionAt(x) { return Math.max(0, Math.min(1, x / track.width)) }
            onPressed: mouse => {
                root.mouseFocus = true
                root.forceActiveFocus(Qt.MouseFocusReason)
                root.dragFraction = fractionAt(mouse.x)
                root.dragging = true
            }
            onPositionChanged: mouse => { if (root.dragging) root.dragFraction = fractionAt(mouse.x) }
            onReleased: mouse => {
                root.dragFraction = fractionAt(mouse.x)
                root.seekRequested(Math.round(root.dragFraction * root.durationMs))
                root.dragging = false
            }
            onCanceled: root.dragging = false
        }
    }

    Accessible.role: Accessible.Slider
    Accessible.name: qsTr("Progreso de la canción")
    Accessible.focusable: enabled && durationMs > 0
    Accessible.description: qsTr("%1 de %2").arg(format(value)).arg(format(maximumValue))
    Accessible.onIncreaseAction: requestSeek(value + stepSize)
    Accessible.onDecreaseAction: requestSeek(value - stepSize)
}
