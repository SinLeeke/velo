import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"
import "../motion"
import ".."

// E5b: DJ Livi in the player bar. Without a DJ session it starts one in Velo (Actions.startDj
// turns Velo's audio on if needed); during one it is «Cambiar de onda» (next DJ block). Filled
// accent glyph while the session runs; a breathing halo (DjHalo) while Livi speaks. Hover or
// keyboard focus shows what Livi is saying (Player.djLine), also exposed to screen readers.
// The whole button is absent when the account cannot use DJ.
Item {
    id: root
    visible: Backend.Player.djAvailable
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight
    readonly property bool live: Backend.Player.djActive
    readonly property bool speaking: live && Backend.Player.djSpeaking
    readonly property string line: speaking ? Backend.Player.djLine : ""

    DjHalo { anchors.fill: parent }
    IconButton {
        id: button
        objectName: "playerBarDj"
        anchors.fill: parent
        text: root.live ? qsTr("Cambiar de onda") : qsTr("Escuchar a DJ Livi")
        iconSource: root.live ? Qt.resolvedUrl("../../icons/record_voice_over_fill.svg") : Qt.resolvedUrl("../../icons/record_voice_over.svg")
        ink: root.live ? Theme.accent : Theme.text
        Accessible.description: root.line
        // C6 language: the glyph pops when the DJ starts and on every «Cambiar de onda».
        onClicked: {
            pop()
            if (root.live) Backend.Actions.djNextSegment()
            else Backend.Actions.startDj()
        }
    }
    ToolTip {
        id: tip
        parent: button
        visible: button.enabled && (button.hovered || button.visualFocus)
        delay: Theme.dj.tipDelay
        y: -height - Theme.space.sm
        padding: Theme.space.md
        // S1-2 menu feel: fade + 0.98 -> 1 from its bottom edge (toward the button), `fast`.
        transformOrigin: Popup.Bottom
        enter: null
        exit: null
        property PopupEntrance entrance: PopupEntrance { popup: tip; fromScale: Theme.entrance.menuScale; duration: Theme.motion.fast }
        contentItem: Column {
            spacing: Theme.space.xs
            LibraryLabel {
                text: button.text
                font.weight: Theme.type.semibold
            }
            LibraryLabel {
                visible: root.line !== ""
                width: Math.min(implicitWidth, Theme.dj.tipWidth)
                text: root.line
                color: Theme.textSecondary
                font.pixelSize: Theme.type.caption
                wrapMode: Text.Wrap
                maximumLineCount: 4
            }
        }
        // R3 (P2-1): same elevated plate as the menus.
        background: SortMenu.PopoverBackground {}
    }
}
