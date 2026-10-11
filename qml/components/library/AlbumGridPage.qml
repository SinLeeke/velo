import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"

Item {
    id: page
    property int requestedAtCount: -1
    function fetchNearEnd() {
        if (visible && Backend.Library.hasMore && !Backend.Library.loading && !failed
                && grid.contentY - grid.originY + grid.height >= grid.contentHeight - grid.cellHeight * 2
                && requestedAtCount !== grid.count) {
            requestedAtCount = grid.count
            Backend.Library.fetchMore()
        }
    }
    // R4 (P1-1): loading, empty and failed are three different states. A failed load shows
    // Rust's message (Library.loadError) and "Reintentar"; while loading there is no empty text.
    property string localLoadError: ""
    readonly property string loadError: "loadError" in Backend.Library && Backend.Library.loadError !== ""
        ? Backend.Library.loadError : localLoadError
    readonly property bool failed: loadError !== "" && !Backend.Library.loading
    function reload() {
        localLoadError = ""
        if (grid.count > 0) { requestedAtCount = grid.count; Backend.Library.fetchMore() }
        else { requestedAtCount = -1; if (discography) Backend.Library.openArtist(artistId); else Backend.Library.openSavedAlbums() }
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
    property bool discography: false
    property string artistId: ""
    signal albumRequested(string albumId)
    LibraryLabel {
        id: heading
        width: parent.width
        text: page.discography ? qsTr("Discografía") : qsTr("Álbumes guardados")
        font.pixelSize: page.discography ? Theme.type.heading : Theme.type.display
        font.letterSpacing: Theme.type.tracking(page.discography ? Theme.type.heading : Theme.type.display)
        font.weight: Theme.type.bold
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    GridView {
        id: grid
        objectName: "albumGrid"
        anchors.top: heading.bottom
        anchors.topMargin: Theme.space.xl
        anchors.bottom: parent.bottom
        // Shifted by the card inset so the art lines up with the page heading.
        x: -Theme.space.sm
        width: parent.width + 2 * Theme.library.cardBleed
        clip: true
        reuseItems: true
        cellWidth: width / Math.max(1, Math.floor(width / Theme.size.coverMin))
        // P2-9: cards grow in Theme.library.gridArtStep steps, so a window resize re-decodes the
        // visible art once per step instead of on every pixel; the cell keeps the slack.
        readonly property real cardWidth: Math.max(Theme.library.gridArtStep,
            Math.floor((cellWidth - Theme.space.lg) / Theme.library.gridArtStep) * Theme.library.gridArtStep)
        cellHeight: cardWidth + Theme.space.lg + Theme.size.trackRow
        model: page.discography ? Backend.Library.albums : Backend.Library.savedAlbums
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
            required property string albumId
            required property string name
            required property string artist
            required property string imageUrl
            required property int year
            width: grid.cardWidth
            artSource: imageUrl
            title: name
            subtitle: year > 0 ? qsTr("%1 · %2").arg(year).arg(artist) : artist
            uri: "spotify:album:" + albumId
            // P1-2: Tab reaches the cards past the visible ones.
            onActiveFocusChanged: if (activeFocus) { grid.currentIndex = index; grid.positionViewAtIndex(index, GridView.Contain) }
            Keys.onPressed: event => grid.tabCard(event, index)
            onClicked: page.albumRequested(albumId)
        }
        footer: Column {
            x: Theme.space.sm
            width: grid.width - 2 * Theme.space.sm
            spacing: Theme.space.md
            LibraryLabel {
                objectName: "albumGridStatus"
                width: parent.width
                visible: Backend.Library.loading || page.failed || grid.count === 0
                text: Backend.Library.loading ? qsTr("Cargando álbumes…") : page.failed ? page.loadError
                    : qsTr("Los álbumes que guardes en Spotify aparecerán aquí.")
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
            LibraryButton { objectName: "albumGridRetry"; visible: page.failed; text: qsTr("Reintentar"); onClicked: page.reload() }
        }
    }
}
