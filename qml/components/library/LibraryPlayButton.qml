import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"

// Round accent play/pause disc of the library player bar.
// Disabled: neutral disc, no hover, whole control at Theme.library.disabledOpacity.
// Keyboard focus (visualFocus only): ring inset by Theme.stroke.focus in
// Theme.library.focusOnAccent, so accent surrounds it on both sides (>= 3:1 in both modes).
// `text` is the accessible name and must be a qsTr() string.
T.AbstractButton {
    id: control
    property url iconSource
    property int iconSize: Theme.icon.md
    readonly property bool live: enabled && (hovered || pressed)
    implicitWidth: Theme.size.iconButton
    implicitHeight: Theme.size.iconButton
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    padding: 0
    opacity: enabled ? 1 : Theme.library.disabledOpacity
    Accessible.name: text
    Accessible.role: Accessible.Button
    // Space activates the focused disc; the window's Space shortcut must not take it first (P1-3).
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()
    ToolTip {
        parent: control
        visible: control.enabled && control.hovered && !control.pressed && control.text !== ""
        delay: Theme.dj.tipDelay
        text: control.text
        padding: Theme.space.sm
        y: -height - Theme.space.sm
        background: SortMenu.PopoverBackground {}
        contentItem: LibraryLabel { text: control.text }
    }
    // Press: the disc settles (scale, instant down / fast back). Hover: the lighter fill fades in
    // over the accent (opacity: instant in, fast out). Transform and opacity only.
    scale: pressed ? Theme.feel.pressScale : 1
    Behavior on scale {
        ScaleAnimator { duration: control.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    background: Rectangle {
        radius: height / 2
        color: !control.enabled ? Theme.surface3 : Theme.accent
        Rectangle {
            anchors.fill: parent
            radius: height / 2
            color: Theme.accentHover
            opacity: control.live ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, control.live ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: Theme.stroke.focus
            radius: height / 2
            visible: control.visualFocus && control.enabled
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.library.focusOnAccent
        }
    }
    contentItem: Item {
        IconImage {
            anchors.centerIn: parent
            source: control.iconSource
            sourceSize: Qt.size(control.iconSize, control.iconSize)
            color: control.enabled ? Theme.accentInk : Theme.textSecondary
        }
    }
}
