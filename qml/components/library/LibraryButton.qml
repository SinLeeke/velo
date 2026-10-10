import QtQuick
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"

// Text button of the library world: sidebar entries (selected = accentSoft), ghost actions,
// and the primary accent action (Reproducir, Iniciar sesión).
// Disabled: no hover/press fill, accent dropped from primary, whole control at
// Theme.library.disabledOpacity. Keyboard focus (visualFocus only, never on mouse press):
// accent border on neutral fills; on the accent fill an inset accentInk ring so the indicator
// is surrounded by accent on both sides (>= 3:1 in both modes, see Theme.library.focusOnAccent).
T.Button {
    id: control
    property url iconSource
    property bool primary: false
    property bool selected: false
    property color restFill: Theme.library.clearPaint
    readonly property bool hasIcon: iconSource.toString() !== ""
    readonly property bool live: enabled && (hovered || down)
    implicitHeight: Theme.size.titleBar
    implicitWidth: measure.advanceWidth + leftPadding + rightPadding + (hasIcon ? Theme.icon.md + spacing : 0)
    leftPadding: Theme.space.lg
    rightPadding: Theme.space.lg
    spacing: Theme.space.sm
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    opacity: enabled ? 1 : Theme.library.disabledOpacity
    Accessible.name: text
    // Space activates the focused button (T.Button), ahead of any window Shortcut.
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    // M1b: Enter activates the focused button too (Space already does), as in dialogs.
    Keys.onReturnPressed: if (enabled) clicked()
    Keys.onEnterPressed: if (enabled) clicked()
    TextMetrics { id: measure; text: control.text; font.family: Theme.family; font.pixelSize: Theme.type.body; font.weight: Theme.type.semibold }
    // Press: the button settles (scale; wide buttons a little less, so it reads the same).
    // Hover and selection: their fills fade in over the resting one (opacity: hover instant in,
    // fast out; selection fast). Transform and opacity only; the ink changes at once.
    readonly property bool hoverShown: live && (primary || !selected)
    scale: down ? (width > Theme.size.coverMin ? Theme.feel.pressScaleWide : Theme.feel.pressScale) : 1
    Behavior on scale {
        ScaleAnimator { duration: control.down ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    background: Rectangle {
        radius: Theme.radius.md
        color: control.primary ? (!control.enabled ? Theme.surface3 : Theme.accent) : control.restFill
        Rectangle {   // selected fill
            anchors.fill: parent
            radius: parent.radius
            color: Theme.accentSoft
            opacity: control.selected && !control.primary ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // hover / pressed fill
            anchors.fill: parent
            radius: parent.radius
            color: control.primary ? Theme.accentHover : Theme.surface3
            opacity: control.hoverShown ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, control.hoverShown ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // keyboard focus ring on neutral fills
            anchors.fill: parent
            radius: parent.radius
            visible: control.visualFocus && !control.primary
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.focusRing
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: Theme.stroke.focus
            radius: parent.radius - Theme.stroke.focus
            visible: control.visualFocus && control.primary && control.enabled
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.library.focusOnAccent
        }
    }
    property color ink: !enabled ? Theme.textSecondary : primary ? Theme.accentInk : selected ? Theme.accent : Theme.text
    contentItem: Row {
        spacing: control.spacing
        IconImage {
            visible: control.hasIcon
            anchors.verticalCenter: parent.verticalCenter
            source: control.iconSource
            sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
            color: control.enabled && !control.primary && !control.selected ? Theme.textSecondary : control.ink
        }
        LibraryLabel {
            anchors.verticalCenter: parent.verticalCenter
            width: Math.max(0, control.width - control.leftPadding - control.rightPadding
                   - (control.hasIcon ? Theme.icon.md + control.spacing : 0))
            text: control.text
            color: control.ink
            font.weight: Theme.type.semibold
        }
    }
}
