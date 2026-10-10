import QtQuick
import QtQuick.Shapes
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"
import "scallop.js" as Scallop

// Play/pause on a scalloped white disc (Theme.scallop.lobesPlay), glyph in cover.controlInk.
T.AbstractButton {
    id: control

    property bool playing: false

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
    HoverHandler {
        id: mouseHover
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
    }
    scale: pressed ? Theme.feel.pressScale : mouseHover.hovered && enabled ? Theme.feel.hoverLift : 1
    Behavior on scale {
        enabled: control.playing && control.visible && control.Window.visibility !== Window.Minimized
            && control.Window.visibility !== Window.Hidden
        ScaleAnimator {
            duration: control.pressed ? Theme.motion.instant : Theme.motion.fast
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.motion.easeOut
        }
    }

    background: Shape {
        id: disc
        preferredRendererType: Shape.CurveRenderer
        property var outline: Scallop.points(width, height, Theme.scallop.lobesPlay, Theme.nowPlaying.scallopDepthPlay)
        ShapePath {
            strokeColor: control.visualFocus ? Theme.cover.controlInk : Theme.library.clearPaint
            strokeWidth: control.visualFocus ? Theme.stroke.focus : -1
            fillColor: control.enabled ? Theme.cover.control : Theme.cover.inkFaint
            PathPolyline { path: disc.outline }
        }
    }

    contentItem: Item {
        IconImage {
            anchors.centerIn: parent
            source: control.playing ? Qt.resolvedUrl("../../icons/pause_fill.svg") : Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
            sourceSize: Qt.size(Theme.icon.play, Theme.icon.play)
            color: Theme.cover.controlInk
        }
    }
}
