import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../nav"

// M1: menu of an album / playlist / artist card, in the official app's order. Library
// membership is asked when the menu opens (Actions.inspectSaved) and the entry stays disabled
// until Spotify answers. Radio and "Copiar enlace" come from LibraryMenu.
LibraryMenu {
    id: menu
    property string uri: ""
    // Own playlists are not "saved": removing one from the library would delete it.
    property bool owned: false
    property string mediaTitle: ""
    property string description: ""
    property string detailsUri: ""
    readonly property bool detailsKnown: detailsUri === uri && uri !== ""
    property string deleteUri: ""
    seedUri: uri
    readonly property string kind: uri.split(":")[1] || ""
    readonly property bool known: Backend.Actions.inspectedUri === uri && Backend.Actions.inspectedSaved >= 0
    readonly property bool saved: known && Backend.Actions.inspectedSaved === 1

    // Keep LibraryMenu's onAboutToShow (share URL) and inspect membership only on opening.
    Connections {
        target: menu
        function onAboutToShow() {
            if (menu.kind !== "" && !menu.owned) Backend.Actions.inspectSaved(menu.uri)
        }
    }
    // A lazy menu reads the playlist roles without assuming QAbstractListModel has get().
    Instantiator {
        id: ownedDetails
        active: menu.owned && menu.kind === "playlist"
        model: Backend.Library.playlists
        delegate: QtObject {
            required property string playlistId
            required property string name
            required property string description
            property bool matches: playlistId === menu.uri.split(":")[2]
            function publish() {
                if (!matches) return
                menu.detailsUri = "spotify:playlist:" + playlistId
                menu.mediaTitle = name
                menu.description = description
            }
            onMatchesChanged: publish()
            onNameChanged: publish()
            onDescriptionChanged: publish()
            Component.onCompleted: publish()
        }
    }

    Action {
        text: qsTr("Editar detalles")
        enabled: menu.owned && menu.kind === "playlist" && menu.detailsKnown
        onTriggered: {
            editLoader.active = true
            const dialog = editLoader.item as PlaylistEditDialog
            // The menu's own content leaves the window once it closes: anchor the dialog to
            // the item the menu was opened from so it can still open.
            dialog.parent = menu.parent
            dialog.openFor(menu.uri.split(":")[2], menu.mediaTitle, menu.description)
        }
    }
    Action {
        text: qsTr("Eliminar")
        enabled: menu.owned && menu.kind === "playlist"
        onTriggered: {
            menu.deleteUri = menu.uri
            deleteLoader.active = true
            const dialog = deleteLoader.item as LibraryConfirmDialog
            dialog.message = qsTr("Esto eliminará «%1» de tu biblioteca.").arg(menu.mediaTitle)
            dialog.parent = menu.parent
            dialog.open()
        }
    }
    Loader { id: editLoader; active: false; sourceComponent: PlaylistEditDialog {} }
    Loader {
        id: deleteLoader
        active: false
        sourceComponent: LibraryConfirmDialog {
            title: qsTr("¿Eliminar de tu biblioteca?")
            confirmText: qsTr("Eliminar")
            onConfirmed: Backend.Actions.setSaved(menu.deleteUri, false)
        }
    }

    Action {
        text: qsTr("Añadir a la cola")
        enabled: menu.kind === "album" || menu.kind === "playlist"
        onTriggered: Backend.Actions.addContextToQueue(menu.uri)
    }
    Action {
        text: menu.kind === "artist" ? (menu.saved ? qsTr("Dejar de seguir") : qsTr("Seguir"))
              : menu.saved ? qsTr("Quitar de tu biblioteca") : qsTr("Guardar en tu biblioteca")
        enabled: menu.known && !menu.owned
        onTriggered: Backend.Actions.setSaved(menu.uri, !menu.saved)
    }
}
