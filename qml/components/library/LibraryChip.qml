import QtQuick
import QtQuick.Templates as T
import "../../theme"

// Filter chip (pill): artist discography types, search result types (F2). Selected = accent
// soft fill with accent ink; idle = surface2. Keyboard focus draws the focus ring.
T.Button {
    id: chip
    property bool selected: false
    readonly property bool live: enabled && (hovered || down)
    implicitHeight: Theme.library.chipHeight
    implicitWidth: measure.advanceWidth + leftPadding + rightPadding
    leftPadding: Theme.space.lg
    rightPadding: Theme.space.lg
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    opacity: enabled ? 1 : Theme.library.disabledOpacity
    checkable: false
    Accessible.role: Accessible.RadioButton
    Accessible.checkable: true
    Accessible.checked: selected
    Accessible.name: text
    // Space activates the focused chip (T.Button), ahead of any window Shortcut.
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    TextMetrics { id: measure; text: chip.text; font.family: Theme.family; font.pixelSize: Theme.type.caption; font.weight: Theme.type.semibold }
    // Press: the chip settles (scale). Selection crossfades the neutral and the accent fills
    // (opacity, fast), the ink too (two labels, accent over neutral); hover fades its fill
// (instant in, fast out). Opacity and scale only.
    scale: down ? Theme.feel.pressScale : 1
    Behavior on scale {
        ScaleAnimator { duration: Math.max(1, chip.down ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    readonly property bool hoverShown: live && !selected
    background: Item {
        Rectangle {   // idle fill
            anchors.fill: parent
            radius: height / 2
            color: Theme.surface2
            opacity: chip.selected ? 0 : 1
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // hover fill
            anchors.fill: parent
            radius: height / 2
            color: Theme.surface3
            opacity: chip.hoverShown ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, chip.hoverShown ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // selected fill
            anchors.fill: parent
            radius: height / 2
            color: Theme.accentSoft
            opacity: chip.selected ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // keyboard focus ring
            anchors.fill: parent
            radius: height / 2
            visible: chip.visualFocus
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.focusRing
        }
    }
    contentItem: Item {
        implicitWidth: idleInk.implicitWidth
        implicitHeight: idleInk.implicitHeight
        LibraryLabel {
            id: idleInk
            anchors.fill: parent
            text: chip.text
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
            opacity: chip.selected ? 0 : 1
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        LibraryLabel {
            anchors.fill: parent
            text: chip.text
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            color: Theme.accent
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
            opacity: chip.selected ? 1 : 0
            Accessible.ignored: true
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
    }
}
