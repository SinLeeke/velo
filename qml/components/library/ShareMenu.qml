import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../nav"

// «…» de pódcast, de episodio y de perfil: Copiar enlace (open.spotify.com) y, si `queueEntry`,
// Añadir a la cola. Sin radio (esos contenidos no la tienen). El enlace se pide a Rust al abrir.
LibraryMenu {
    id: menu
    objectName: "shareMenu"
    property string uri: ""
    // Episodios: ofrecer «Añadir a la cola».
    property bool queueEntry: false
    seedActions: false
    property string link: ""
    // Connections (not onAboutToShow:) so LibraryMenu's own handler keeps running.
    Connections {
        target: menu
        function onAboutToShow() { menu.link = menu.uri !== "" ? Backend.Actions.shareUrl(menu.uri) : "" }
    }

    Action {
        objectName: "shareQueueAction"
        text: qsTr("Añadir a la cola")
        enabled: menu.queueEntry && menu.uri !== ""
        onTriggered: Backend.Library.addToQueue(menu.uri)
    }
    Action {
        objectName: "shareCopyAction"
        text: qsTr("Copiar enlace")
        enabled: menu.link !== ""
        onTriggered: Nav.copy(menu.link)
    }
}
