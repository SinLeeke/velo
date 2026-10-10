import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import QtQuick.Controls.impl
import Velo.Backend
import "../theme"
import "controls"
import "motion"

// "Add to playlist" popup. Public API: `uri`, openFor(uri), close() (and opened from T.Popup).
// Lists Library.playlists, picks one with click/Enter -> Library.addToPlaylist(playlistId, uri),
// and lets the user create a new playlist (Library.createPlaylist(name)) from the footer field.
// Wire-up (leader): the library emits addToPlaylistRequested(uri) -> picker.openFor(uri).
// Position it with x/y relative to its parent (anchored near the clicked row, or centered).
// No timers. Motion (S1-2): fades in + scales 0.97 -> 1 from the centre, leaves shorter.
T.Popup {
    id: root

    property string uri: ""

    readonly property int maxRows: Theme.panel.pickerRows

    function openFor(trackUri) {
        uri = trackUri
        newName.text = ""
        open()
    }

    width: Theme.panel.pickerWidth
    height: contentItem.implicitHeight + topPadding + bottomPadding   // a bare Popup computes 0 for a Column content
    padding: Theme.space.sm
    modal: false
    focus: true
    closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
    enter: null
    property PopupEntrance entrance: PopupEntrance { popup: root }
    exit: PopupExitTransition {}

    onOpened: {
        list.keyboardNav = false
        list.forceActiveFocus()
    }

    function createNew() {
        const name = newName.text.trim()
        if (name === "")
            return
        Library.createPlaylist(name)
        newName.text = ""
    }

    background: Rectangle {
        radius: Theme.radius.lg
        color: Theme.surface1
        border.width: Theme.stroke.hairline
        border.color: Theme.hairline

        RectangularShadow {
            z: -1   // behind the card
            anchors.fill: parent
            radius: parent.radius
            offset.y: Theme.elevation.popover.offsetY
            blur: Theme.elevation.popover.blur
            spread: Theme.elevation.popover.spread
            color: Theme.elevation.popoverShadow
        }
    }

    contentItem: Column {
        spacing: 0

        Text {
            x: Theme.space.md
            height: implicitHeight + 2 * Theme.space.sm
            verticalAlignment: Text.AlignVCenter
            text: qsTr("Añadir a una playlist")
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
            color: Theme.textSecondary
            Accessible.role: Accessible.Heading
        }

        ListView {
            id: list
            width: parent.width
            height: Math.min(contentHeight, root.maxRows * Theme.panel.pickerRow)
            visible: count > 0
            clip: true
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds
            model: Library.playlists
            keyNavigationEnabled: true
            property bool keyboardNav: false   // focus ring only once the keyboard is used
            Keys.onPressed: keyboardNav = true
            currentIndex: 0

            Accessible.role: Accessible.List
            Accessible.name: qsTr("Tus playlists")

            Keys.onReturnPressed: if (currentItem) currentItem.activate()
            Keys.onEnterPressed: if (currentItem) currentItem.activate()
            Keys.onSpacePressed: if (currentItem) currentItem.activate()

            delegate: T.AbstractButton {
                id: item
                required property var model
                required property int index

                width: ListView.view.width
                height: Theme.panel.pickerRow
                hoverEnabled: true
                focusPolicy: Qt.NoFocus
                padding: 0
                text: model.name

                Accessible.name: text
                Accessible.role: Accessible.Button

                function activate() {
                    Library.addToPlaylist(model.playlistId, root.uri)
                    root.close()
                }
                onClicked: activate()

                background: Rectangle {
                    radius: Theme.radius.md
                    color: item.hovered || item.pressed ? Theme.surface2 : Theme.library.clearPaint
                    border.width: item.ListView.isCurrentItem && list.keyboardNav ? Theme.stroke.focus : 0
                    border.color: Theme.focusRing
                }
                contentItem: Item {
                    Rectangle {
                        id: plate
                        anchors.left: parent.left
                        anchors.leftMargin: Theme.space.sm
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.size.iconButton + Theme.space.xs
                        height: width
                        radius: Theme.radius.sm
                        color: Theme.surface3
                        IconImage {
                            anchors.centerIn: parent
                            source: Qt.resolvedUrl("../icons/queue_music.svg")
                            sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                            color: Theme.textSecondary
                        }
                    }
                    Column {
                        anchors.left: plate.right
                        anchors.leftMargin: Theme.space.md
                        anchors.right: parent.right
                        anchors.rightMargin: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.space.xxs
                        Text {
                            width: parent.width
                            text: item.text
                            elide: Text.ElideRight
                            font.family: Theme.family
                            font.pixelSize: Theme.type.body
                            font.weight: Theme.type.semibold
                            color: Theme.text
                        }
                        Text {
                            width: parent.width
                            text: qsTr("%n canción(es)", "", item.model.trackCount)
                            elide: Text.ElideRight
                            font.family: Theme.family
                            font.pixelSize: Theme.type.caption
                            color: Theme.textSecondary
                        }
                    }
                }
            }
        }

        Text {
            visible: list.count === 0
            x: Theme.space.md
            width: parent.width - 2 * Theme.space.md
            height: visible ? implicitHeight + 2 * Theme.space.md : 0
            wrapMode: Text.WordWrap
            text: qsTr("Aún no tienes playlists. Crea la primera aquí abajo.")
            font.family: Theme.family
            font.pixelSize: Theme.type.body
            lineHeight: Theme.type.bodyLineHeight
            color: Theme.textTertiary
        }

        Rectangle {   // divider
            width: parent.width - 2 * Theme.space.sm
            x: Theme.space.sm
            height: Theme.stroke.hairline
            color: Theme.hairline
        }

        Item {   // new playlist
            width: parent.width
            height: newName.implicitHeight + 2 * Theme.space.sm
            VTextField {
                id: newName
                anchors.left: parent.left
                anchors.leftMargin: Theme.space.xs
                anchors.right: addButton.left
                anchors.rightMargin: Theme.space.sm
                anchors.verticalCenter: parent.verticalCenter
                placeholderText: qsTr("Nueva playlist")
                onAccepted: root.createNew()
            }
            IconButton {
                id: addButton
                anchors.right: parent.right
                anchors.rightMargin: Theme.space.xs
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Crear playlist")
                iconSource: Qt.resolvedUrl("../icons/add.svg")
                iconSize: Theme.icon.md
                enabled: newName.text.trim() !== ""
                ink: Theme.accent
                hoverFill: Theme.accentSoft
                onClicked: root.createNew()
            }
        }
    }
}
