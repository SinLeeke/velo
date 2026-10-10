import QtQuick
import Velo.Backend as Backend
import "../../theme"

// M1b "Editar detalles" of an own playlist: name and description, Guardar / Cancelar.
// Enter in a field saves, Esc cancels. Rust trims and ignores an empty name; the sidebar row
// and the open header change when Spotify accepts it (errors arrive through Player.error).
LibraryDialog {
    id: dialog
    objectName: "playlistEditDialog"
    property string playlistId: ""
    // Spotify's own limits for these fields.
    readonly property int nameMax: Theme.library.playlistNameMax
    readonly property int descriptionMax: Theme.library.playlistDescriptionMax
    readonly property bool canSave: nameField.text.trim() !== ""

    function openFor(id, name, description) {
        playlistId = id
        nameField.text = name
        descriptionField.text = description
        open()
    }
    function save() {
        if (!canSave) return
        Backend.Library.updatePlaylistDetails(playlistId, nameField.text, descriptionField.text)
        close()
    }

    title: qsTr("Editar detalles")
    initialFocus: nameField
    onOpened: nameField.selectAll()

    LibraryLabel {
        width: parent.width
        text: qsTr("Nombre")
        color: Theme.textSecondary
        font.pixelSize: Theme.type.caption
        font.weight: Theme.type.semibold
    }
    LibraryField {
        id: nameField
        objectName: "playlistNameField"
        width: parent.width
        placeholderText: qsTr("Añade un nombre")
        maximumLength: dialog.nameMax
        Accessible.name: qsTr("Nombre")
        onAccepted: dialog.save()
    }
    LibraryLabel {
        width: parent.width
        text: qsTr("Descripción")
        color: Theme.textSecondary
        font.pixelSize: Theme.type.caption
        font.weight: Theme.type.semibold
    }
    LibraryField {
        id: descriptionField
        objectName: "playlistDescriptionField"
        width: parent.width
        placeholderText: qsTr("Añade una descripción opcional")
        maximumLength: dialog.descriptionMax
        Accessible.name: qsTr("Descripción")
        onAccepted: dialog.save()
    }
    LibraryLabel {
        width: parent.width
        text: qsTr("Al guardar, el cambio se aplica en Spotify para todos tus dispositivos.")
        color: Theme.textSecondary
        font.pixelSize: Theme.type.caption
        wrapMode: Text.Wrap
    }

    buttons: [
        LibraryButton { text: qsTr("Cancelar"); onClicked: dialog.close() },
        LibraryButton { objectName: "playlistSaveButton"; primary: true; text: qsTr("Guardar"); enabled: dialog.canSave; onClicked: dialog.save() }
    ]
}
