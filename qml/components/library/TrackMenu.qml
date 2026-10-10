import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../nav"
import "creditsMemo.js" as CreditsMemo

// M1 / R4: the song menu, in the official app's order (radio and "Copiar enlace" come from
// LibraryMenu). One instance per list page, shared by all its TrackRows: a row fills it with its
// song when it opens it (openFor), so a long list does not build ~10 menu entries per row.
LibraryMenu {
    id: menu
    objectName: "trackContextMenu"
    // The row the menu was opened for (its signals are forwarded through it).
    property var row: null
    property string uri: ""
    property bool liked: false
    property string artistId: ""
    property string albumId: ""
    property string playlistId: ""
    property bool ownPlaylist: false
    property bool creditsKnownEmpty: false
    seedUri: uri
    onClosed: row = null

    function fill(source) {
        row = source
        uri = source.uri
        liked = source.liked
        artistId = source.artistId
        albumId = source.albumId
        playlistId = source.playlistId
        ownPlaylist = source.ownPlaylist
        creditsKnownEmpty = CreditsMemo.isEmpty(source.uri)
    }
    function openFor(source, x, y) {
        fill(source)
        popup(source, x, y)
    }
    Connections {
        target: Backend.Actions
        function onSavedChanged(uri, saved) { if (uri === menu.uri) menu.liked = saved }
    }

    Action { text: qsTr("Añadir a playlist…"); onTriggered: if (menu.row) menu.row.addToPlaylistRequested(menu.uri) }
    Action {
        text: qsTr("Quitar de esta playlist")
        enabled: menu.ownPlaylist && menu.playlistId !== ""
        onTriggered: Backend.Library.removeFromPlaylist(menu.playlistId, menu.uri)
    }
    Action {
        text: menu.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Guardar en Tus me gusta")
        onTriggered: Backend.Actions.setSaved(menu.uri, !menu.liked)
    }
    Action { text: qsTr("Añadir a la cola"); onTriggered: Backend.Library.addToQueue(menu.uri) }
    Action { text: qsTr("Ir al artista"); enabled: menu.artistId !== ""; onTriggered: Nav.show("artist", menu.artistId) }
    Action { text: qsTr("Ir al álbum"); enabled: menu.albumId !== ""; onTriggered: Nav.show("album", menu.albumId) }
    // E6: credits from Spotify's track metadata (CreditsDialog). A song whose credits came back
    // empty in this session does not offer the entry again.
    Action {
        text: qsTr("Ver créditos")
        enabled: menu.uri.startsWith("spotify:track:") && !menu.creditsKnownEmpty
        onTriggered: Nav.showCredits(menu.uri)
    }
}
