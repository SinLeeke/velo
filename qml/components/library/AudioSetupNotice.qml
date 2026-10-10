import QtQuick
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"

// E2b existente: autorización de audio una sola vez; credenciales en el keyring.
Rectangle {
    id: notice
    property string errorMessage: ""
    readonly property var window: notice.Window.window
    readonly property bool effectsAllowed: visible && Theme.motion.scale > 0
        && (!window || (window.visibility !== Window.Minimized && window.visibility !== Window.Hidden
            && (window.active || !Theme.pauseEffectsInBackground)))
    implicitHeight: body.height + 2 * Theme.space.lg
    color: Theme.accentSoft; radius: Theme.radius.lg
    transform: Translate { id: shift }
    FirstUseReveal { id: entrance; item: notice; shift: shift; effectsAllowed: notice.effectsAllowed }
    Component.onCompleted: entrance.begin()
    Connections {
        target: Backend.Account
        function onAudioFailed(reason) { notice.errorMessage = reason }
        function onAudioBusyChanged() { if (Backend.Account.audioBusy) notice.errorMessage = "" }
    }
    Column {
        id: body
        x: Theme.space.lg; y: x; width: parent.width - 2 * x
        spacing: Theme.space.sm
        Row {
            width: parent.width; spacing: Theme.space.sm
            IconImage { source: Qt.resolvedUrl("../../icons/computer.svg"); sourceSize: Qt.size(Theme.icon.md, Theme.icon.md); color: Theme.accent }
            LibraryLabel {
                width: parent.width - Theme.icon.md - Theme.space.sm
                text: Backend.Account.audioBusy ? qsTr("Activa el audio en tu navegador") : qsTr("Escucha desde este PC")
                font.pixelSize: Theme.type.title; font.weight: Theme.type.semibold; wrapMode: Text.Wrap
            }
        }
        LibraryLabel {
            width: parent.width
            text: Backend.Account.audioBusy
                ? qsTr("Termina la autorización de audio en Spotify y vuelve a Velo.")
                : qsTr("Tu biblioteca ya está lista. Autoriza el audio una vez para escuchar aquí con Spotify Premium.")
            color: Theme.textSecondary; wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
        }
        LibraryButton {
            id: audioButton
            objectName: "enableAudioFirstUse"
            width: Math.min(implicitWidth, parent.width)
            height: Math.max(implicitHeight, audioActionText.implicitHeight + 2 * Theme.space.md)
            text: qsTr("Activar la reproducción en este PC")
            primary: true; enabled: !Backend.Account.audioBusy
            iconSource: Qt.resolvedUrl("../../icons/play_arrow.svg")
            onClicked: { notice.errorMessage = ""; Backend.Account.enableAudio() }
            // Con la cola abierta queda poco ancho: mantener la acción completa y clicable.
            contentItem: Row {
                spacing: audioButton.spacing
                IconImage {
                    anchors.verticalCenter: parent.verticalCenter
                    source: audioButton.iconSource
                    sourceSize: Qt.size(Theme.icon.md, Theme.icon.md); color: audioButton.ink
                }
                LibraryLabel {
                    id: audioActionText
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(0, audioButton.width - audioButton.leftPadding - audioButton.rightPadding - Theme.icon.md - audioButton.spacing)
                    text: audioButton.text; color: audioButton.ink; font.weight: Theme.type.semibold
                    wrapMode: Text.Wrap
                }
            }
        }
        LibraryLabel {
            width: parent.width; visible: notice.errorMessage !== ""
            text: notice.errorMessage; color: Theme.danger
            wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
            Accessible.role: Accessible.AlertMessage
        }
    }
}
