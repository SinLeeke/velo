import QtQuick
import Velo.Backend as Backend
import "../theme"
import "library"

// A persistent companion to Toast: the user decides when an available update goes away.
Rectangle {
    id: root
    objectName: "updateNotice"
    visible: Backend.Updater.state === "available" || Backend.Updater.state === "downloading"
        || Backend.Updater.state === "ready" || Backend.Updater.state === "error"
    width: Math.min(parent.width - 2 * Theme.space.gutter, Theme.toast.width)
    height: body.height + 2 * Theme.space.md
    radius: Theme.radius.lg
    color: Theme.surface2
    border.width: Theme.stroke.hairline
    border.color: Backend.Updater.state === "error" ? Theme.danger : Theme.controlBorder
    Accessible.role: Accessible.AlertMessage
    Accessible.name: message.text
    Column {
        id: body
        x: Theme.space.md; y: Theme.space.md
        width: parent.width - 2 * Theme.space.md
        spacing: Theme.space.sm
        LibraryLabel {
            id: message
            objectName: "updateMessage"
            width: parent.width
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: Backend.Updater.state === "error" ? Backend.Updater.error
                : Backend.Updater.state === "downloading" ? qsTr("Descargando… %1 %").arg(Math.round(Backend.Updater.progress * 100))
                : Backend.Updater.state === "ready" ? qsTr("Abriendo el instalador…")
                : qsTr("Velo %1 disponible").arg(Backend.Updater.latestVersion)
            Accessible.ignored: true
        }
        Rectangle {
            visible: Backend.Updater.state === "downloading"
            width: parent.width; height: Theme.stroke.focus
            radius: Theme.radius.sm
            color: Theme.surface3
            Rectangle {
                width: parent.width * Math.max(0, Math.min(1, Backend.Updater.progress))
                height: parent.height; radius: parent.radius; color: Theme.accent
            }
        }
        Row {
            visible: Backend.Updater.state === "available" || Backend.Updater.state === "error"
            spacing: Theme.space.sm
            LibraryButton {
                objectName: "updateInstall"
                primary: true
                text: Backend.Updater.state === "error" ? qsTr("Reintentar") : qsTr("Actualizar")
                onClicked: Backend.Updater.install()
            }
            LibraryButton {
                objectName: "updateDismiss"
                text: qsTr("Más tarde")
                onClicked: Backend.Updater.dismiss()
            }
        }
    }
}
