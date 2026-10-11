import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import ".."
import "../motion"

// Windows system output (separate from Spotify Connect devices). Event-driven model.
IconButton {
    id: root
    readonly property alias outputPopup: popup
    text: qsTr("Salida de audio del sistema")
    iconSource: Qt.resolvedUrl("../../icons/speaker.svg")
    active: popup.opened
    Accessible.checkable: true
    Accessible.checked: popup.opened
    Accessible.description: qsTr("Cambia la salida predeterminada de Windows")
    onClicked: popup.opened ? popup.close() : popup.open()

    T.Popup {
        id: popup
        x: root.width - width
        y: -height - Theme.space.sm
        width: Theme.panel.devicePopoverWidth
        height: contentItem.implicitHeight + topPadding + bottomPadding
        padding: Theme.space.sm
        focus: true
        modal: false
        // Outside its parent (this button): a press on the button itself does not close it from
        // "outside" first, so the click toggles it shut instead of reopening it.
        closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutsideParent
        // Motion (S1-2): like the menus: fade + scale 0.98 -> 1 from the bottom-right corner (its
        // button), `fast`; closing is immediate, so focus is back on the button at once.
        // Rows change instantly.
        transformOrigin: T.Popup.BottomRight
        enter: null
        property PopupEntrance entrance: PopupEntrance { popup: root.outputPopup; fromScale: Theme.entrance.menuScale; duration: Theme.motion.fast }
        exit: null
        onAboutToShow: Backend.Player.refreshOutputs()
        onOpened: {
            list.currentIndex = 0
            for (let i = 0; i < Backend.Player.outputs.count; ++i)
                if (Backend.Player.outputs.get(i).isDefault) list.currentIndex = i
            list.keyboardNav = false
            list.forceActiveFocus()
        }
        onClosed: root.forceActiveFocus(Qt.OtherFocusReason)
        background: Rectangle {
            color: Theme.surface1
            radius: Theme.radius.lg
            border.width: Theme.stroke.hairline
            border.color: Theme.hairline
            RectangularShadow {
                z: -1
                anchors.fill: parent
                radius: parent.radius
                offset.y: Theme.elevation.popover.offsetY
                blur: Theme.elevation.popover.blur
                spread: Theme.elevation.popover.spread
                color: Theme.audioOutput.shadow
            }
        }
        contentItem: Column {
            spacing: Theme.space.xs
            LibraryLabel {
                x: Theme.space.md
                width: parent.width - 2 * x
                height: implicitHeight + 2 * Theme.space.sm
                verticalAlignment: Text.AlignVCenter
                text: qsTr("Salida de audio del sistema")
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
                color: Theme.textSecondary
                Accessible.role: Accessible.Heading
            }
            ListView {
                id: list
                objectName: "systemAudioOutputList"
                width: parent.width
                height: Math.min(contentHeight, Theme.audioOutput.maxRows * Theme.panel.deviceRow)
                clip: true
                reuseItems: true
                boundsBehavior: Flickable.StopAtBounds
                model: Backend.Player.outputs
                property bool keyboardNav: false
                keyNavigationEnabled: true
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Salidas de audio del sistema")
                Keys.onPressed: keyboardNav = true
                Keys.onShortcutOverride: event => {
                    if ([Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter, Qt.Key_Escape].indexOf(event.key) >= 0) event.accepted = true
                }
                Keys.onReturnPressed: if (currentItem) currentItem.activate()
                Keys.onEnterPressed: if (currentItem) currentItem.activate()
                Keys.onSpacePressed: if (currentItem) currentItem.activate()
                Keys.onEscapePressed: popup.close()
                delegate: T.AbstractButton {
                    id: output
                    required property int index
                    required property string outputId
                    required property string name
                    required property bool isDefault
                    objectName: "systemAudioOutputRow" + index
                    width: list.width
                    height: Theme.panel.deviceRow
                    hoverEnabled: true
                    text: name
                    Accessible.name: name
                    Accessible.description: isDefault ? qsTr("Predeterminada del sistema") : qsTr("Usar como salida predeterminada del sistema")
                    Accessible.role: Accessible.RadioButton
                    Accessible.checked: isDefault
                    Accessible.checkable: true
                    function activate() { Backend.Player.setOutput(outputId); popup.close() }
                    onClicked: activate()
                    background: Rectangle {
                        radius: Theme.radius.md
                        color: output.isDefault ? Theme.accentSoft : output.hovered || output.pressed ? Theme.surface2 : Theme.library.clearPaint
                        border.width: output.ListView.isCurrentItem && list.keyboardNav ? Theme.stroke.focus : 0
                        border.color: Theme.focusRing
                    }
                    contentItem: Item {
                        IconImage {
                            id: speaker
                            anchors.left: parent.left
                            anchors.leftMargin: Theme.space.md
                            anchors.verticalCenter: parent.verticalCenter
                            source: root.iconSource
                            sourceSize: Qt.size(Theme.icon.lg, Theme.icon.lg)
                            color: output.isDefault ? Theme.accent : Theme.textSecondary
                        }
                        Column {
                            anchors.left: speaker.right
                            anchors.leftMargin: Theme.space.md
                            anchors.right: check.left
                            anchors.rightMargin: Theme.space.sm
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: Theme.space.xxs
                            LibraryLabel { width: parent.width; text: output.name; font.weight: Theme.type.semibold; color: output.isDefault ? Theme.accent : Theme.text }
                            LibraryLabel { visible: output.isDefault; width: parent.width; text: qsTr("Predeterminada del sistema"); font.pixelSize: Theme.type.caption; color: Theme.textSecondary }
                        }
                        IconImage {
                            id: check
                            anchors.right: parent.right
                            anchors.rightMargin: Theme.space.md
                            anchors.verticalCenter: parent.verticalCenter
                            visible: output.isDefault
                            source: Qt.resolvedUrl("../../icons/check.svg")
                            sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                            color: Theme.accent
                        }
                    }
                }
            }
            LibraryLabel {
                x: Theme.space.md
                width: parent.width - 2 * x
                visible: list.count === 0
                text: qsTr("No hay salidas de audio disponibles.")
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
        }
    }
}
