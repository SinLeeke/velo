import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"

// Play/pause on a clean round disc (cover.control) with a soft RectangularShadow and, only while
// music plays, a faint halo ring that breathes around it. The user asked for a plain circle or a
// design of our own instead of the scalloped "flower"; this is the own design: a quiet disc whose
// only ornament is the breathing ring (ambient, so it stops when paused/minimised/Modo rendimiento).
// Motion (opacity + scale only, all retargetable):
//  - press: the whole control settles to pressScale in `instant` (<= 90 ms), back in `fast`;
//  - hover (mouse only): lifts to hoverLift;
//  - play: the face pops (1 -> 1.1 -> 1) when playback starts;
//  - play <-> pause: the two glyphs swap by crossfade + scale from toolMarkFrom (`fast`, easeOut).
T.AbstractButton {
    id: control

    property bool playing: false

    readonly property real popScale: Theme.feel.discPop
    readonly property int haloHalf: Theme.motion.haloHalf
    readonly property real haloMaxScale: Theme.feel.haloMaxScale
    readonly property real haloPeak: Theme.feel.haloPeak
    readonly property bool haloOn: playing && enabled && visible && Theme.effectsAllowed
        && Theme.motion.scale > 0 && Window.window !== null && Window.window.visible
        && Window.window.visibility !== Window.Minimized
    readonly property bool animatable: visible && Window.window !== null && Window.window.visible
        && Window.window.visibility !== Window.Minimized && Window.window.visibility !== Window.Hidden

    implicitWidth: Theme.size.playDisc
    implicitHeight: Theme.size.playDisc
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    padding: 0
    text: playing ? qsTr("Pausar") : qsTr("Reproducir")
    Accessible.name: text
    Accessible.role: Accessible.Button
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()
    NowPlayingToolTip { button: control }

    // Transform only: a mouse pointer lifts the disc (not touch or keyboard focus) and a press
    // settles it. Down is quick (instant), the way back a little slower (fast); retargets mid-way.
    // Interaction motion: it works also with the music paused (the leader's rule 1).
    HoverHandler {
        id: mouseHover
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
    }
    scale: pressed ? Theme.feel.pressScale : mouseHover.hovered && enabled ? Theme.feel.hoverLift : 1
    Behavior on scale {
        enabled: control.animatable
        ScaleAnimator {
            duration: Math.max(1, control.pressed ? Theme.motion.instant : Theme.motion.fast)
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.motion.easeOut
        }
    }

    onPlayingChanged: if (playing && animatable && Theme.motion.scale > 0) facePop.restart()

    background: Item {
        // Halo: a ring that breathes (scale + opacity, looping) only while music plays. Its own
        // wrapper fades out when the breath stops, so it never freezes half-way on screen.
        Item {
            id: halo
            anchors.fill: parent
            opacity: control.haloOn ? 1 : 0
            visible: opacity > 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            Rectangle {
                id: ring
                anchors.fill: parent
                radius: width / 2
                color: Theme.library.clearPaint
                border.width: Theme.stroke.focus
                border.color: Theme.cover.control
                opacity: 0
                SequentialAnimation {
                    running: control.haloOn
                    loops: Animation.Infinite
                    ParallelAnimation {
                        ScaleAnimator { target: ring; from: 1; to: control.haloMaxScale; duration: control.haloHalf; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                        OpacityAnimator { target: ring; from: 0; to: control.haloPeak; duration: control.haloHalf; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                    }
                    ParallelAnimation {
                        ScaleAnimator { target: ring; from: control.haloMaxScale; to: 1; duration: control.haloHalf; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                        OpacityAnimator { target: ring; from: control.haloPeak; to: 0; duration: control.haloHalf; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                    }
                }
            }
        }

        // The face (disc + shadow + glyphs) owns the pop; the control itself owns press/hover.
        Item {
            id: face
            anchors.fill: parent
            RectangularShadow {
                z: -1
                anchors.fill: disc
                radius: disc.radius
                offset.y: Theme.elevation.raised.offsetY
                blur: Theme.elevation.raised.blur
                spread: Theme.elevation.raised.spread
                color: Qt.rgba(0, 0, 0, control.enabled ? Theme.elevation.raised.alpha : 0)
            }
            Rectangle {
                id: disc
                anchors.fill: parent
                radius: width / 2
                color: control.enabled ? Theme.cover.control : Theme.cover.inkFaint
            }
            // Keyboard focus: a ring just outside the disc, in the cover ink (never animated).
            Rectangle {
                anchors.fill: parent
                anchors.margins: -2 * Theme.stroke.focus
                visible: control.visualFocus
                radius: width / 2
                color: Theme.library.clearPaint
                border.width: Theme.stroke.focus
                border.color: Theme.cover.ink
            }
            SequentialAnimation {
                id: facePop
                ScaleAnimator { target: face; from: 1; to: control.popScale; duration: Math.max(1, Theme.motion.instant); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                ScaleAnimator { target: face; to: 1; duration: Math.max(1, Theme.motion.base - Theme.motion.instant); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }
        }
    }

    contentItem: Item {
        // Two stacked glyphs swap by crossfade + scale (opacity and transform only).
        component Glyph: IconImage {
            id: glyph
            property bool shown
            anchors.centerIn: parent
            sourceSize: Qt.size(Theme.icon.play, Theme.icon.play)
            color: Theme.cover.controlInk
            opacity: shown ? 1 : 0
            scale: shown ? 1 : Theme.feel.toolMarkFrom
            visible: opacity > 0
            Behavior on opacity {
                enabled: control.animatable
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            Behavior on scale {
                enabled: control.animatable
                ScaleAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Glyph {
            objectName: "playGlyph"
            shown: !control.playing
            source: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
        }
        Glyph {
            objectName: "pauseGlyph"
            shown: control.playing
            source: Qt.resolvedUrl("../../icons/pause_fill.svg")
        }
    }
}
