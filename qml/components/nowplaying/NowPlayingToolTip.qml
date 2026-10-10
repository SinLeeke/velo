import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Templates as T
import QtQuick.Effects
import Velo.Backend
import "../../theme"

// Shared by the controls in R2's surface. A paused tooltip opens without a delay timer;
// hidden/minimized windows never keep one waiting. No animated popup or auto-hide timer.
ToolTip {
    id: tip
    property T.AbstractButton button: null
    parent: button
    readonly property bool windowShown: button !== null && button.Window.window !== null
        && button.Window.visibility !== Window.Minimized && button.Window.visibility !== Window.Hidden
    visible: windowShown && button.enabled && button.hovered && !button.pressed && text !== ""
    text: button ? button.text : ""
    delay: Player.isPlaying ? Theme.cover.tipDelay : 0
    timeout: -1
    padding: Theme.space.md
    enter: null
    exit: null
    contentItem: Text {
        text: tip.text
        textFormat: Text.PlainText
        font.family: Theme.family
        font.pixelSize: Theme.type.caption
        color: Theme.text
    }
    background: Rectangle {
        radius: Theme.radius.md
        color: Theme.surface2
        border.width: Theme.stroke.hairline
        border.color: Theme.hairline
        RectangularShadow {
            z: -1
            anchors.fill: parent
            radius: parent.radius
            offset.y: Theme.elevation.popover.offsetY
            blur: Theme.elevation.popover.blur
            spread: Theme.elevation.popover.spread
            color: Qt.rgba(0, 0, 0, Theme.elevation.popover.alpha)
        }
    }
}
