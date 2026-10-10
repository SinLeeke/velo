import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import "../../theme"
import "../motion"

// Modal dialog of the library world (M1b, E6): scrim over the window, centered surface2 card
// with a RectangularShadow, title, free content and a right-aligned button row.
// A modal Popup is a tab fence, so Tab stays inside; Esc closes (closePolicy). Children go into
// the body (default property); buttons into `buttons`. `initialFocus` gets focus on open.
// Motion (S1-2): the scrim fades and the card fades + scales 0.97 -> 1 from the centre (base,
// ease-out); it leaves shorter (fast, ease-in, -> 0.98). Inherited by every dialog of the library.
T.Popup {
    id: dialog
    property string title: ""
    property Item initialFocus: null
    // TOKEN: library.dialogWidth (readable form width, a bit wider than the playlist picker)
    readonly property int preferredWidth: Theme.panel.pickerWidth + Theme.space.xxxl * 2
    default property alias content: body.data
    property alias buttons: buttonRow.data

    parent: T.Overlay.overlay
    x: parent ? Math.round((parent.width - width) / 2) : 0
    y: parent ? Math.round((parent.height - height) / 2) : 0
    width: parent ? Math.min(preferredWidth, parent.width - 2 * Theme.space.gutter) : preferredWidth
    // A bare Popup computes 0 for a Column content (same note as PlaylistPicker).
    height: column.implicitHeight + topPadding + bottomPadding
    padding: Theme.space.xl
    modal: true
    focus: true
    closePolicy: T.Popup.CloseOnEscape
    enter: null
    property PopupEntrance entrance: PopupEntrance { popup: dialog }
    exit: PopupExitTransition {}
    onOpened: if (initialFocus) initialFocus.forceActiveFocus(Qt.TabFocusReason)

    // The popup writes the dimmer's opacity (0 on hide, 1 on show); the Behavior turns that into
    // a fade on the render thread, timed like the card.
    T.Overlay.modal: Rectangle {
        color: Theme.scrim
        Behavior on opacity {
            id: scrimFade
            OpacityAnimator {
                duration: Math.max(1, scrimFade.targetValue > 0 ? Theme.motion.base : Theme.motion.fast)
                easing.type: Easing.Bezier
                easing.bezierCurve: scrimFade.targetValue > 0 ? Theme.motion.easeOut : Theme.motion.easeIn
            }
        }
    }

    background: Rectangle {
        radius: Theme.radius.lg
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
            color: Theme.elevation.popoverShadow
        }
    }

    contentItem: Item {
        implicitHeight: column.implicitHeight
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.title
        Column {
            id: column
            width: parent.width
            spacing: Theme.space.lg
            LibraryLabel {
                width: parent.width
                text: dialog.title
                font.pixelSize: Theme.type.heading
                font.weight: Theme.type.bold
                wrapMode: Text.Wrap
                maximumLineCount: 2
                Accessible.role: Accessible.Heading
            }
            Column {
                id: body
                width: parent.width
                spacing: Theme.space.md
            }
            Row {
                id: buttonRow
                anchors.right: parent.right
                spacing: Theme.space.sm
                visible: children.length > 0
            }
        }
    }
}
