import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"

Item {
    id: page
    property int requestedAtCount: -1
    property string localLoadError: ""
    readonly property string loadError: "loadError" in Backend.Library && Backend.Library.loadError !== ""
        ? Backend.Library.loadError : localLoadError
    readonly property bool failed: loadError !== "" && !Backend.Library.loading
    function reload() {
        localLoadError = ""
        if (grid.count > 0) { requestedAtCount = grid.count; Backend.Library.fetchMore() }
        else { requestedAtCount = -1; Backend.Library.openFollowedArtists() }
    }
    function fetchNearEnd() {
        if (visible && !failed && Backend.Library.hasMore && !Backend.Library.loading
                && grid.contentY - grid.originY + grid.height >= grid.contentHeight - grid.cellHeight * 2
                && requestedAtCount !== grid.count) {
            requestedAtCount = grid.count
            Backend.Library.fetchMore()
        }
    }
    onVisibleChanged: { if (visible) Qt.callLater(fetchNearEnd) }
    Connections {
        target: Backend.Library
        function onCurrentKindChanged() { page.localLoadError = ""; page.requestedAtCount = -1; grid.positionViewAtBeginning() }
        function onLoadingChanged() { if (Backend.Library.loading) page.localLoadError = ""; else Qt.callLater(page.fetchNearEnd) }
    }
    Connections {
        target: Backend.Player
        function onError(message) { if (page.visible && Backend.Library.loading) page.localLoadError = message }
    }
    signal artistRequested(string artistId)
    LibraryLabel { id: heading; width: parent.width; text: qsTr("Artistas que sigues"); font.pixelSize: Theme.type.display; font.letterSpacing: Theme.type.tracking(Theme.type.display); font.weight: Theme.type.bold; Accessible.role: Accessible.Heading; Accessible.name: text }
    GridView {
        id: grid
        objectName: "artistGrid"
        anchors.top: heading.bottom
        anchors.topMargin: Theme.space.xl
        anchors.bottom: parent.bottom
        // Shifted by the card inset so the art lines up with the page heading.
        x: -Theme.space.sm
        width: parent.width + 2 * Theme.library.cardBleed
        clip: true
        reuseItems: true
        cellWidth: width / Math.max(1, Math.floor(width / Theme.size.coverMin))
        readonly property real cardWidth: Math.max(Theme.library.gridArtStep,
            Math.floor((cellWidth - Theme.space.lg) / Theme.library.gridArtStep) * Theme.library.gridArtStep)
        cellHeight: cardWidth + Theme.space.lg + Theme.size.trackRow
        model: Backend.Library.followedArtists
        onContentYChanged: page.fetchNearEnd()
        onCountChanged: Qt.callLater(page.fetchNearEnd)
        onHeightChanged: Qt.callLater(page.fetchNearEnd)
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        // Motion (G2, A-P2): one fade of the whole grid when the first albums/artists arrive late.
        FirstDataFade { target: grid; count: grid.count; allowed: page.visible }
        function tabCard(event, fromIndex) {
            const backwards = event.key === Qt.Key_Backtab || (event.key === Qt.Key_Tab && (event.modifiers & Qt.ShiftModifier))
            if (event.key !== Qt.Key_Tab && !backwards) return
            const next = fromIndex + (backwards ? -1 : 1)
            if (next < 0 || next >= count) return
            currentIndex = next
            positionViewAtIndex(next, GridView.Contain)
            forceLayout()
            const item = itemAtIndex(next)
            if (item) { item.forceActiveFocus(backwards ? Qt.BacktabFocusReason : Qt.TabFocusReason); event.accepted = true }
        }
        delegate: MediaCard {
            required property int index
            required property string artistId
            required property string name
            required property string imageUrl
            width: grid.cardWidth
            artSource: imageUrl
            title: name
            subtitle: qsTr("Artista")
            portrait: true
            uri: "spotify:artist:" + artistId
            onActiveFocusChanged: if (activeFocus) { grid.currentIndex = index; grid.positionViewAtIndex(index, GridView.Contain) }
            Keys.onPressed: event => grid.tabCard(event, index)
            onClicked: page.artistRequested(artistId)
        }
        footer: Column {
            x: Theme.space.sm
            width: grid.width - 2 * Theme.space.sm
            spacing: Theme.space.md
            LibraryLabel {
                objectName: "artistGridStatus"
                width: parent.width
                visible: Backend.Library.loading || page.failed || grid.count === 0
                text: Backend.Library.loading ? qsTr("Cargando artistas…") : page.failed ? page.loadError : qsTr("Los artistas que sigas aparecerán aquí.")
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
            LibraryButton { objectName: "artistGridRetry"; visible: page.failed; text: qsTr("Reintentar"); onClicked: page.reload() }
        }
    }
}
