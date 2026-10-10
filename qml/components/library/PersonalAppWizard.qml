import QtQuick
import Velo.Backend as Backend
import "../../theme"

// Adaptación del asistente personal de Spotifast, MIT © 2026 Carmine Paolino.
Column {
    id: root
    objectName: "personalAppWizard"
    spacing: Theme.space.md
    property string errorMessage: ""
    property bool expanded: false
    readonly property string redirectUri: "http://127.0.0.1:8989/login"
    readonly property bool validId: /^[a-fA-F0-9]{32}$/.test(clientId.text.trim())
    Row {
        width: parent.width
        spacing: Theme.space.md
        LibraryLabel {
            width: Math.max(0, parent.width - configure.width - parent.spacing)
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Usar mi propia app de Spotify")
            font.pixelSize: Theme.type.title
            font.weight: Theme.type.semibold
            wrapMode: Text.Wrap
        }
        LibraryButton {
            id: configure
            objectName: "configurePersonalApp"
            anchors.verticalCenter: parent.verticalCenter
            text: root.expanded ? qsTr("Ocultar") : qsTr("Configurar…")
            Accessible.name: root.expanded ? qsTr("Ocultar configuración de app de Spotify") : qsTr("Configurar mi app de Spotify")
            onClicked: root.expanded = !root.expanded
        }
    }
    Column {
        width: parent.width
        spacing: Theme.space.md
        visible: root.expanded
        LibraryLabel {
            width: parent.width
            color: Theme.textSecondary
            wrapMode: Text.Wrap
            text: qsTr("Tu app tiene su propia cuota. Spotify puede pedir Premium para crearla. Autoriza con la misma cuenta que usas en Velo.")
        }
        LibraryButton {
            text: qsTr("1. Abrir Spotify for Developers")
            width: parent.width
            iconSource: Qt.resolvedUrl("../../icons/open_in_new.svg")
            onClicked: Qt.openUrlExternally("https://developer.spotify.com/dashboard")
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            text: qsTr("2. Pulsa Create app y escribe un nombre y una descripción.")
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            text: qsTr("3. En Redirect URIs pega exactamente esta dirección:")
        }
        LibraryField {
            id: redirect
            width: parent.width
            readOnly: true
            text: root.redirectUri
            selectByMouse: true
            Accessible.name: qsTr("Redirect URI exacto")
        }
        LibraryButton {
            text: qsTr("Copiar dirección")
            onClicked: {
                redirect.selectAll();
                redirect.copy();
                redirect.deselect();
            }
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            text: qsTr("4. Marca Web API, acepta las condiciones y guarda.")
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            text: qsTr("5. Abre Settings de tu app, copia el Client ID y pégalo aquí. No necesitas el Client Secret.")
        }
        LibraryField {
            id: clientId
            objectName: "personalClientId"
            width: parent.width
            placeholderText: qsTr("Client ID: 32 caracteres hexadecimales")
            text: Backend.Account.clientId
            enabled: !Backend.Account.busy
            onAccepted: if (root.validId && !Backend.Account.busy)
                authorize.clicked()
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            color: Theme.danger
            visible: clientId.text !== "" && !root.validId
            text: qsTr("El Client ID debe tener exactamente 32 caracteres: 0–9 y a–f.")
        }
        LibraryButton {
            id: authorize
            objectName: "authorizePersonalApp"
            width: parent.width
            primary: true
            text: Backend.Account.busy ? qsTr("Esperando autorización…") : qsTr("6. Autorizar")
            enabled: root.validId && !Backend.Account.busy
            onClicked: {
                root.errorMessage = "";
                Backend.Account.authorizeClient(clientId.text.trim());
            }
        }
        LibraryButton {
            visible: Backend.Account.busy
            text: qsTr("Cancelar autorización")
            onClicked: Backend.Account.cancelSignIn()
        }
        LibraryLabel {
            width: parent.width
            wrapMode: Text.Wrap
            visible: root.errorMessage !== ""
            color: Theme.danger
            text: root.errorMessage
        }
        Connections {
            enabled: root.expanded
            target: Backend.Account
            function onSignInFailed(reason) {
                root.errorMessage = reason;
            }
        }
    }
}
