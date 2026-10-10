import QtQuick
import ".."
import Velo.Backend as Backend
import "../../theme"

// Speaker icon (click = mute/unmute) + volume line with a vertical-bar knob (ref_01).
// B1: `volume` is what the backend reports (it lags and echoes stale values). What the control
// SHOWS and accumulates on is `shown`: the value of the user's own gesture (drag, wheel, keys),
// held in `pending` until the backend reports that same value (or `settle` runs out), so the
// knob never jumps back to an old echo, on release either. Every change is sent at once
// (the backend coalesces) and the last one goes out again on release. The "last audible volume"
// is Player.lastAudibleVolume (no copy here).
Item {
    id: root


    property int volume: 0
    property bool canControl: true
    signal volumeRequested(int v)
    // T4: with trackInset >= 0 the track runs from trackInset to width - trackInset (the progress
    // bar's track above, SeekBar.trackInset) and the speaker sits centered in the left time
    // column, so the row is centered under the progress bar. -1 keeps the plain layout
    // (speaker at the left edge, track to the right edge).
    property real trackInset: -1
    readonly property bool aligned: trackInset >= 0

    property bool dragging: false
    property int pending: -1                 // the gesture's latest value, -1 when none is held
    readonly property int shown: pending >= 0 ? pending : volume
    readonly property real fraction: Math.max(0, Math.min(1, shown / 100))

    implicitHeight: Theme.size.iconButton
    readonly property bool settleAllowed: Backend.Player.isPlaying && visible && Window.window !== null
        && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
    onSettleAllowedChanged: {
        if (settleAllowed && pending >= 0) settle.restart()
        else settle.stop()
    }

    function requestVolume(v) {
        if (!enabled || !canControl)
            return
        const next = Math.max(0, Math.min(100, Math.round(v)))
        pending = next
        if (settleAllowed) settle.restart()
        volumeRequested(next)
    }
    function release() {
        dragging = false
        // The last order already went out with the release position (onReleased pushes it).
        if (pending >= 0 && volume === pending) {
            pending = -1
            settle.stop()
        }
    }
    // The observed value caught up with the gesture: follow the backend again (never mid-drag,
    // or an older echo arriving later would be shown).
    onVolumeChanged: if (!dragging && pending >= 0 && volume === pending) { pending = -1; settle.stop() }
    // Gives up on a value the backend never confirms (rejected, clamped). Runs only for the
    // moments after a change, matching the bridge's optimistic hold; nothing runs at rest.
    Timer { id: settle; objectName: "volumeSettle"; interval: Theme.nowPlaying.volumeSettleMs; onTriggered: if (!root.dragging) root.pending = -1 }

    // Mouse wheel: vertical only. Deltas add up and every wheel notch (120) is one volumeStep,
    // so a touchpad's small high-resolution deltas move the volume 1 % at a time.
    property real wheelRest: 0
    function wheelBy(dy) {
        if (!enabled || !canControl || !isFinite(dy) || dy === 0)
            return
        wheelRest += dy * Theme.sliderInput.volumeStep / Theme.nowPlaying.wheelNotch
        const whole = wheelRest > 0 ? Math.floor(wheelRest) : Math.ceil(wheelRest)
        if (whole === 0)
            return
        wheelRest -= whole
        requestVolume(shown + whole)
    }

    IconButton {
        id: speaker
        x: root.aligned ? Math.round((root.trackInset - Theme.space.md - width) / 2) : 0
        anchors.verticalCenter: parent.verticalCenter
        width: Theme.size.iconButton - Theme.space.xs
        height: width
        enabled: root.canControl
        text: root.shown === 0 ? qsTr("Activar sonido") : qsTr("Silenciar")
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        NowPlayingToolTip { button: speaker }
        iconSource: root.shown === 0 ? Qt.resolvedUrl("../../icons/volume_off.svg")
                  : root.shown < 50 ? Qt.resolvedUrl("../../icons/volume_down.svg") : Qt.resolvedUrl("../../icons/volume_up.svg")
        iconSize: Theme.icon.sm
        ink: Theme.coverInkSecondary
        inkDisabled: Theme.cover.inkFaint
        hoverFill: Theme.cover.pillHover
        onClicked: {
            root.requestVolume(root.shown > 0 ? 0 : Backend.Player.lastAudibleVolume)
        }
    }

    Item {
        id: track
        anchors.left: root.aligned ? parent.left : speaker.right
        anchors.leftMargin: root.aligned ? root.trackInset : Theme.space.sm
        anchors.right: parent.right
        anchors.rightMargin: root.aligned ? root.trackInset : knob.width / 2
        anchors.verticalCenter: parent.verticalCenter
        height: Theme.icon.sm

        readonly property real headX: Math.round(width * root.fraction)

        enabled: root.canControl
        activeFocusOnTab: true
        // Focus taken with the mouse shows no ring (like Control.visualFocus); Tab shows it.
        property bool mouseFocus: false
        onActiveFocusChanged: if (!activeFocus) mouseFocus = false
        readonly property int value: Math.round(root.fraction * 100)
        readonly property int minimumValue: 0
        readonly property int maximumValue: 100
        readonly property int stepSize: Theme.sliderInput.volumeStep
        Accessible.role: Accessible.Slider
        Accessible.name: qsTr("Volumen")
        Accessible.focusable: root.canControl
        Accessible.description: qsTr("%1%").arg(value)
        Accessible.onIncreaseAction: root.requestVolume(value + stepSize)
        Accessible.onDecreaseAction: root.requestVolume(value - stepSize)
        Keys.onPressed: event => {
            if (!root.canControl || root.dragging || event.modifiers !== Qt.NoModifier)
                return
            switch (event.key) {
            case Qt.Key_Right: case Qt.Key_Up: root.requestVolume(value + stepSize); break
            case Qt.Key_Left: case Qt.Key_Down: root.requestVolume(value - stepSize); break
            case Qt.Key_PageUp: root.requestVolume(value + Theme.sliderInput.volumePage); break
            case Qt.Key_PageDown: root.requestVolume(value - Theme.sliderInput.volumePage); break
            case Qt.Key_Home: root.requestVolume(minimumValue); break
            case Qt.Key_End: root.requestVolume(maximumValue); break
            default: return
            }
            event.accepted = true
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: -Theme.space.xs
            radius: Theme.radius.xs
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.cover.ink
            visible: track.activeFocus && !track.mouseFocus
            // Keyboard focus ring: shown at once on purpose (it must track fast Tab presses; a fade would lag).
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            x: track.headX
            width: track.width - x
            height: Theme.stroke.progress
            radius: height / 2
            color: Theme.cover.inkFaint
        }
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: track.headX
            height: Theme.stroke.progress
            radius: height / 2
            color: root.canControl ? Theme.cover.ink : Theme.cover.inkFaint
        }
        // Vertical bar knob.
        Rectangle {
            id: knob
            anchors.verticalCenter: parent.verticalCenter
            x: track.headX - width / 2
            width: Theme.stroke.progress
            height: Theme.icon.sm
            radius: width / 2
            color: root.canControl ? Theme.cover.ink : Theme.cover.inkFaint
        }

        MouseArea {
            anchors.fill: parent
            anchors.topMargin: -Theme.space.sm
            anchors.bottomMargin: -Theme.space.sm
            anchors.leftMargin: -knob.width
            anchors.rightMargin: -knob.width
            enabled: root.canControl
            preventStealing: true
            function fractionAt(x) { return Math.max(0, Math.min(1, (x - knob.width) / track.width)) }
            function push(x) { root.requestVolume(Math.round(fractionAt(x) * 100)) }
            onPressed: mouse => {
                track.mouseFocus = true
                track.forceActiveFocus(Qt.MouseFocusReason)
                root.dragging = true
                push(mouse.x)
            }
            onPositionChanged: mouse => { if (root.dragging) push(mouse.x) }
            onReleased: mouse => { push(mouse.x); root.release() }
            onCanceled: root.release()
            onWheel: wheel => {
                const dy = wheel.inverted ? -wheel.angleDelta.y : wheel.angleDelta.y
                if (dy === 0) {
                    wheel.accepted = false   // sideways scrolling is not for this control
                    return
                }
                root.wheelBy(dy)
            }
        }
    }

}
