import QtQuick
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"
import "../../nav"
import "creditsMemo.js" as CreditsMemo

// E6 "Ver créditos": asks Actions.requestCredits(uri) and shows what Spotify's track metadata
// has (artists, composers, producers…). Sections with no data stay hidden. One instance per view, opened through
// Nav.showCredits(uri) from any song menu. No timer: an error (Player.error) while loading
// shows the failure state; closing simply drops a late answer.
LibraryDialog {
    id: dialog
    objectName: "creditsDialog"
    property string uri: ""
    // "loading" | "ready" | "failed"
    property string status: "loading"
    property string trackTitle: ""
    property string failure: ""
    property var contributors: []
    readonly property int maxRows: Theme.library.creditsMaxRows

    function roleLabel(role, title) {
        switch (role) {
        case "main_artist": return qsTr("Artista principal")
        case "featured_artist": return qsTr("Artista invitado")
        case "remixer": return qsTr("Remezcla")
        case "composer": return qsTr("Compositor")
        case "conductor": return qsTr("Director")
        case "orchestra": return qsTr("Orquesta")
        case "actor": return qsTr("Intérprete")
        case "songwriter": return qsTr("Compositor")
        case "producer": return qsTr("Producción")
        case "engineer": return qsTr("Ingeniería de sonido")
        case "mixing_engineer": return qsTr("Mezcla")
        case "mastering_engineer": return qsTr("Masterización")
        default: return title || qsTr("Artista")
        }
    }
    function openFor(trackUri) {
        uri = trackUri
        status = "loading"
        trackTitle = ""
        failure = ""
        contributors = []
        open()
        Backend.Actions.requestCredits(trackUri)
    }
    function accept(json) {
        let data = null
        try { data = JSON.parse(json) } catch (e) { data = null }
        if (!data || !Array.isArray(data.contributors)) { failure = qsTr("La respuesta no se pudo leer. Reintenta la consulta."); status = "failed"; return }
        trackTitle = data.title || ""
        contributors = data.contributors.filter(c => c && c.name).map(c => ({ name: c.name, role: dialog.roleLabel(c.role, c.roleTitle) }))
        status = "ready"
        if (contributors.length === 0) {
            CreditsMemo.markEmpty(uri)
            close()
        }
    }

    title: qsTr("Créditos")
    closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
    initialFocus: closeButton
    onClosed: uri = ""

    Connections {
        target: Nav
        function onCreditsRequested(trackUri) { dialog.openFor(trackUri) }
    }
    Connections {
        target: Backend.Actions
        // The newest request replaces older ones; answers for another song are ignored.
        function onCreditsReady(trackUri, json) { if (dialog.visible && trackUri === dialog.uri) dialog.accept(json) }
    }
    Connections {
        target: Backend.Player
        enabled: dialog.visible && dialog.status === "loading"
        function onError(message) { dialog.failure = message; dialog.status = "failed" }
    }

    LibraryLabel {
        width: parent.width
        visible: dialog.trackTitle !== ""
        text: dialog.trackTitle
        font.pixelSize: Theme.type.title
        font.weight: Theme.type.semibold
    }
    LibraryLabel {
        objectName: "creditsStatus"
        width: parent.width
        visible: dialog.status !== "ready"
        text: dialog.status === "loading" ? qsTr("Cargando…")
              : dialog.status === "failed" ? (dialog.failure !== "" ? qsTr("No se pudieron cargar los créditos: %1").arg(dialog.failure)
                                                                    : qsTr("No se pudieron cargar los créditos."))
              : ""
        color: Theme.textSecondary
        wrapMode: Text.Wrap
        lineHeight: Theme.type.bodyLineHeight
    }
    ListView {
        id: list
        objectName: "creditsList"
        width: parent.width
        visible: dialog.status === "ready" && count > 0
        height: Math.min(count, dialog.maxRows) * Theme.panel.pickerRow
        clip: true
        reuseItems: true
        interactive: count > dialog.maxRows
        boundsBehavior: Flickable.StopAtBounds
        model: dialog.contributors
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Créditos")
        T.ScrollBar.vertical: LibraryScrollBar {}
        delegate: Item {
            required property var modelData
            width: ListView.view.width
            height: Theme.panel.pickerRow
            Accessible.role: Accessible.ListItem
            Accessible.name: qsTr("%1, %2").arg(modelData.name).arg(modelData.role)
            Column {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.space.xxs
                LibraryLabel { width: parent.width; text: modelData.name; font.weight: Theme.type.semibold }
                LibraryLabel { width: parent.width; text: modelData.role; color: Theme.textSecondary; font.pixelSize: Theme.type.caption }
            }
        }
    }

    buttons: [
        LibraryButton { objectName: "creditsRetryButton"; visible: dialog.status === "failed"; text: qsTr("Reintentar"); onClicked: dialog.openFor(dialog.uri) },
        LibraryButton { id: closeButton; objectName: "creditsCloseButton"; text: qsTr("Cerrar"); onClicked: dialog.close() }
    ]
}
