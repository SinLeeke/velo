import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"

// F1 + F2 + T3: the detailed search results, like the official app. The query is typed in the
// library's top bar (SearchBox.qml: suggestions drop under it over any page); Enter there opens
// this page, which shows type chips (Todo, Canciones, Artistas, Playlists, Álbumes). "Todo" is
// "Mejor resultado" + the first songs, then shelves of artists, albums and playlists; a type chip
// shows that list alone (songs reuse the track list, the rest a card grid). The best result is
// Velo's heuristic . The page has no field of its own.
Item {
    id: page
    property string query: ""
    readonly property alias resultPage: songs
    signal querySubmitted(string query)
    signal addToPlaylistRequested(string uri)
    signal navigate(string kind, string itemId, string owner)
    readonly property string kind: Backend.Library.searchType
    readonly property bool hasResults: Backend.Library.searchQuery !== ""
    // R4 (P1-6): one song menu for the "Canciones" rows of "Todo".
    TrackMenu { id: songMenu }

    function kindLabel(k) {
        switch (k) {
        case "track": return qsTr("Canción")
        case "artist": return qsTr("Artista")
        case "album": return qsTr("Álbum")
        case "playlist": return qsTr("Playlist")
        default: return ""
        }
    }
    // Runs the full search for `text` on this page (the top bar's Enter, Reintentar, a song card).
    function submit(text) {
        const q = (text === undefined ? query : text).trim()
        Backend.Library.cancelSearchSuggestions()
        query = q
        querySubmitted(q)
        Backend.Library.search(q)
        songs.resetResults()
    }
    // Opens a result row: artists, albums and playlists navigate; a song runs the full search
    // for it, so "Mejor resultado" shows it with its play button.
    function openRow(row) {
        if (row.kind === "track") submit(row.subtitle !== "" ? row.title + " " + row.subtitle : row.title)
        else navigate(row.kind, row.objectId, row.kind === "playlist" ? row.subtitle : "")
    }
    Component.onCompleted: {
        if (query !== "" && Backend.Library.searchQuery !== query) Backend.Library.search(query)
    }

    // ---- results
    Flow {
        id: chips
        width: parent.width
        anchors.top: parent.top
        visible: page.hasResults
        spacing: Theme.space.sm
        Repeater {
            model: [
                { kind: "all", label: qsTr("Todo") },
                { kind: "track", label: qsTr("Canciones") },
                { kind: "artist", label: qsTr("Artistas") },
                { kind: "playlist", label: qsTr("Playlists") },
                { kind: "album", label: qsTr("Álbumes") }
            ]
            delegate: LibraryChip {
                required property var modelData
                text: modelData.label
                selected: page.kind === modelData.kind
                onClicked: Backend.Library.setSearchType(modelData.kind)
            }
        }
    }
    // Searching / failed. A failed search says why (Rust's message) and offers to run it again.
    readonly property bool searchFailed: Backend.Library.searchError !== "" && !Backend.Library.searchLoading
    Column {
        id: status
        anchors.top: chips.bottom
        anchors.topMargin: Theme.space.lg
        width: parent.width
        spacing: Theme.space.md
        z: 1
        visible: page.hasResults && page.kind !== "track" && (page.searchFailed || (Backend.Library.searchLoading && allView.empty))
        LibraryLabel {
            objectName: "searchStatus"
            width: parent.width
            text: page.searchFailed ? Backend.Library.searchError : qsTr("Buscando…")
            color: page.searchFailed ? Theme.text : Theme.textSecondary
            wrapMode: Text.Wrap
        }
        LibraryButton {
            objectName: "searchRetryButton"
            visible: page.searchFailed
            text: qsTr("Reintentar")
            onClicked: page.submit(Backend.Library.searchQuery)
        }
    }

    // "Todo": best result + songs, then shelves.
    Flickable {
        id: allView
        readonly property bool empty: bestRepeater.count === 0 && songList.count === 0
            && artistShelf.count === 0 && albumShelf.count === 0 && playlistShelf.count === 0
        anchors.top: status.visible ? status.bottom : chips.bottom
        anchors.topMargin: Theme.space.xl
        anchors.bottom: parent.bottom
        x: -Theme.library.cardBleed
        width: page.width + 2 * Theme.library.cardBleed
        visible: page.hasResults && page.kind === "all"
        contentWidth: width
        contentHeight: allContent.height + Theme.space.xl
        flickableDirection: Flickable.VerticalFlick
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        ScrollBar.vertical: LibraryScrollBar {}
        // P1-2: keyboard focus brings the focused card or row into view (no scroll animation).
        function revealItem(item) {
            const top = item.mapToItem(contentItem, 0, 0).y
            const bottom = top + item.height
            const target = top < contentY ? top : bottom > contentY + height ? bottom - height : contentY
            contentY = Math.max(originY, Math.min(target, originY + Math.max(0, contentHeight - height)))
        }
        function revealCard(list, index) {
            list.positionViewAtIndex(index, ListView.Contain)
            revealItem(list)
        }
        function scrollWheel(delta) {
            contentY = Math.max(originY, Math.min(contentY - delta / 120 * Theme.size.trackRow,
                originY + Math.max(0, contentHeight - height)))
        }
        function tabCard(list, event, fromIndex) {
            const backwards = event.key === Qt.Key_Backtab || (event.key === Qt.Key_Tab && (event.modifiers & Qt.ShiftModifier))
            if (event.key !== Qt.Key_Tab && !backwards) return
            const next = fromIndex + (backwards ? -1 : 1)
            if (next < 0 || next >= list.count) return
            list.currentIndex = next
            list.positionViewAtIndex(next, ListView.Contain)
            list.forceLayout()
            const item = list.itemAtIndex(next)
            if (item) { item.forceActiveFocus(backwards ? Qt.BacktabFocusReason : Qt.TabFocusReason); event.accepted = true }
        }
        Column {
            id: allContent
            x: Theme.library.cardBleed
            width: page.width - Theme.space.md
            spacing: Theme.space.xxl
            // U4: "Mejor resultado" and "Canciones" side by side like Spotify: both headings on
            // one line, the card exactly as tall as the 4 song rows and aligned to their top.
            Grid {
                id: topGrid
                width: parent.width
                readonly property bool sideBySide: width >= Theme.search.sideBySideWidth
                readonly property real bestWidth: sideBySide ? Math.round(width * Theme.search.bestShare) : width
                columns: sideBySide ? 2 : 1
                columnSpacing: Theme.space.xl
                rowSpacing: Theme.space.xl
                visible: bestRepeater.count > 0 || songList.count > 0
                Column {
                    width: topGrid.bestWidth
                    spacing: Theme.space.md
                    visible: bestRepeater.count > 0
                    SectionTitle { text: qsTr("Mejor resultado") }
                    Repeater {
                        id: bestRepeater
                        model: Backend.Library.searchBestResult
                        delegate: T_BestCard {}
                    }
                }
                Column {
                    width: topGrid.sideBySide ? topGrid.width - topGrid.bestWidth - topGrid.columnSpacing : topGrid.width
                    spacing: Theme.space.md
                    visible: songList.count > 0
                    SectionTitle { text: qsTr("Canciones") }
                    ListView {
                        id: songList
                        objectName: "searchSongList"
                        width: parent.width
                        height: Math.min(count, Theme.search.songsShown) * Theme.size.trackRow
                        interactive: false
                        clip: true
                        reuseItems: true
                        model: Backend.Library.tracks
                        // Only the columns that fit beside the card: title/artist, heart,
                        // duration and "..." (no "#", album or date).
                        delegate: TrackRow {
                            width: songList.width
                            visible: index < Theme.search.songsShown
                            numberWidth: 0
                            titleWidth: Math.max(0, songList.width - 2 * Theme.trackList.rowInset - Theme.trackList.rowArt
                                        - Theme.trackList.trailingWidth - 4 * Theme.space.md)
                            albumWidth: 0
                            addedWidth: 0
                            sharedMenu: songMenu
                            keyboardCount: Math.min(songList.count, Theme.search.songsShown)
                            onActiveFocusChanged: if (activeFocus) { songList.currentIndex = index; allView.revealItem(this) }
                            onAddToPlaylistRequested: uri => page.addToPlaylistRequested(uri)
                        }
                    }
                }
            }
            SearchShelf { id: artistShelf; heading: qsTr("Artistas"); resultType: "artist"; model: Backend.Library.searchArtists; portrait: true }
            SearchShelf { id: albumShelf; heading: qsTr("Álbumes"); resultType: "album"; model: Backend.Library.searchAlbums }
            SearchShelf { id: playlistShelf; heading: qsTr("Playlists"); resultType: "playlist"; model: Backend.Library.searchPlaylists }
            LibraryLabel {
                objectName: "searchEmpty"
                visible: allView.empty && !Backend.Library.searchLoading && Backend.Library.searchError === ""
                width: parent.width
                text: qsTr("No encontramos nada para «%1». Revisa la ortografía o prueba con menos palabras.").arg(Backend.Library.searchQuery)
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
        }
    }

    // "Canciones": the full track list (Library.tracks mirrors the search songs, pages included).
    TrackListPage {
        id: songs
        anchors.top: status.visible ? status.bottom : chips.bottom
        anchors.topMargin: Theme.space.xl
        anchors.bottom: parent.bottom
        width: parent.width
        visible: page.hasResults && page.kind === "track"
        searchResults: true
        onAddToPlaylistRequested: uri => page.addToPlaylistRequested(uri)
    }

    // "Artistas" / "Álbumes" / "Playlists": card grid with paging near the end.
    GridView {
        id: grid
        objectName: "searchGrid"
        anchors.top: status.visible ? status.bottom : chips.bottom
        anchors.topMargin: Theme.space.xl
        anchors.bottom: parent.bottom
        x: -Theme.space.sm
        width: parent.width + 2 * Theme.library.cardBleed
        visible: page.hasResults && page.kind !== "all" && page.kind !== "track"
        clip: true
        reuseItems: true
        cellWidth: width / Math.max(1, Math.floor(width / Theme.size.coverMin))
        // P2-9: cards grow in Theme.library.gridArtStep steps, so a window resize re-decodes the
        // visible art once per step instead of on every pixel; the cell keeps the slack.
        readonly property real cardWidth: Math.max(Theme.library.gridArtStep,
            Math.floor((cellWidth - Theme.space.lg) / Theme.library.gridArtStep) * Theme.library.gridArtStep)
        cellHeight: cardWidth + Theme.space.lg + Theme.size.trackRow
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        model: visible ? Backend.Library.searchResults : null
        property int requestedAtCount: -1
        function fetchNearEnd() {
            if (visible && !page.searchFailed && Backend.Library.searchHasMore && !Backend.Library.searchLoading
                    && contentY - originY + height >= contentHeight - cellHeight * 2 && requestedAtCount !== count) {
                requestedAtCount = count
                Backend.Library.fetchMoreSearch()
            }
        }
        onContentYChanged: fetchNearEnd()
        onCountChanged: { if (count === 0) requestedAtCount = -1; Qt.callLater(fetchNearEnd) }
        onHeightChanged: Qt.callLater(fetchNearEnd)
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
            required property string kind
            required property string objectId
            required property string uri
            required property string imageUrl
            required property var model
            width: grid.cardWidth
            // P1-2: Tab reaches the cards past the visible ones.
            onActiveFocusChanged: if (activeFocus) { grid.currentIndex = index; grid.positionViewAtIndex(index, GridView.Contain) }
            Keys.onPressed: event => grid.tabCard(event, index)
            artSource: imageUrl
            title: model.title
            subtitle: model.subtitle !== "" ? model.subtitle : page.kindLabel(kind)
            portrait: kind === "artist"
            uri: model.uri
            onClicked: page.openRow({ kind: kind, objectId: objectId, title: model.title, subtitle: model.subtitle })
        }
        LibraryLabel {
            x: Theme.space.sm
            width: grid.width - 2 * Theme.space.sm
            visible: grid.count === 0 && !Backend.Library.searchLoading && !page.searchFailed
            text: qsTr("No hay resultados de este tipo.")
            color: Theme.textSecondary
        }
    }

    // No query (only reachable through an old history entry): point at the field above.
    LibraryLabel {
        objectName: "searchNoQuery"
        width: parent.width
        visible: !page.hasResults
        text: qsTr("Escribe en el buscador de arriba lo que quieres escuchar.")
        color: Theme.textSecondary
        wrapMode: Text.Wrap
    }

    // P2-6: section titles are headings for Narrator.
    component SectionTitle: LibraryLabel {
        font.pixelSize: Theme.type.heading
        font.letterSpacing: Theme.type.tracking(Theme.type.heading)
        font.weight: Theme.type.semibold
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    // Horizontal shelf of result cards ("Todo").
    component SearchShelf: Column {
        id: shelf
        property string heading
        property string resultType
        property alias model: shelfList.model
        property bool portrait: false
        readonly property alias count: shelfList.count
        width: parent ? parent.width : 0
        spacing: Theme.space.md
        visible: count > 0
        Row {
            width: parent.width
            SectionTitle { width: Math.max(0, parent.width - showAll.implicitWidth - Theme.space.md); text: shelf.heading }
            LibraryButton { id: showAll; text: qsTr("Mostrar todo"); onClicked: Backend.Library.setSearchType(shelf.resultType) }
        }
        ListView {
            id: shelfList
            objectName: "searchShelf_" + shelf.resultType
            x: -Theme.space.sm
            width: shelf.width + 2 * Theme.library.cardBleed
            height: Theme.library.cardWidth + Theme.size.trackRow + Theme.space.md
            orientation: ListView.Horizontal
            spacing: Theme.space.md
            reuseItems: true
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.horizontal: LibraryScrollBar { height: Theme.space.sm; policy: shelfList.contentWidth > shelfList.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff }
            WheelHandler { onWheel: event => { if (event.angleDelta.y !== 0) { allView.scrollWheel(event.angleDelta.y); event.accepted = true } } }
            delegate: MediaCard {
                required property int index
                required property string kind
                required property string objectId
                required property string imageUrl
                required property var model
                width: Theme.library.cardWidth
                onActiveFocusChanged: if (activeFocus) { shelfList.currentIndex = index; allView.revealCard(shelfList, index) }
                Keys.onPressed: event => allView.tabCard(shelfList, event, index)
                artSource: imageUrl
                title: model.title
                subtitle: model.subtitle !== "" ? model.subtitle : page.kindLabel(kind)
                portrait: shelf.portrait
                uri: model.uri
                onClicked: page.openRow({ kind: kind, objectId: objectId, title: model.title, subtitle: model.subtitle })
            }
        }
    }

    // "Mejor resultado" (U4, like Spotify): uniform padding; the art (round for an artist), the
    // title under it and "Artista" / "Canción • ..."; the green play disc rises in at the bottom
    // right while the card is hovered or focused (opacity + y, `fast`, ease-out; render thread).
    component T_BestCard: LibraryButton {
        id: best
        objectName: "searchBestCard"
        required property string kind
        required property string objectId
        required property string title
        required property string subtitle
        required property string imageUrl
        readonly property bool revealed: enabled && (hovered || visualFocus || playBest.hovered || playBest.activeFocus)
        width: parent ? parent.width : 0
        height: Theme.search.songsShown * Theme.size.trackRow
        padding: Theme.search.bestPadding
        leftPadding: Theme.search.bestPadding
        rightPadding: Theme.search.bestPadding
        text: qsTr("%1, %2").arg(title).arg(page.kindLabel(kind))
        background: Rectangle {
            radius: Theme.radius.lg
            color: Theme.surface2
            Rectangle {   // hover fill fades over the resting one (opacity only)
                anchors.fill: parent
                radius: parent.radius
                color: Theme.surface3
                opacity: best.revealed ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, best.revealed ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }
            border.width: best.visualFocus ? Theme.stroke.focus : 0
            border.color: Theme.focusRing
        }
        contentItem: Item {
            LibraryArtwork {
                id: bestArt
                width: Theme.search.bestArt
                height: width
                source: best.imageUrl
                portrait: best.kind === "artist"
                fallback: best.kind === "artist" ? Qt.resolvedUrl("../../icons/person.svg")
                        : best.kind === "track" ? Qt.resolvedUrl("../../icons/music_note.svg") : Qt.resolvedUrl("../../icons/library_music.svg")
            }
            LibraryLabel {
                id: bestTitle
                anchors.top: bestArt.bottom
                anchors.topMargin: Theme.space.lg
                width: parent.width
                text: best.title
                font.pixelSize: Theme.type.display
                font.weight: Theme.type.bold
            }
            LibraryLabel {
                anchors.top: bestTitle.bottom
                anchors.topMargin: Theme.space.xs
                width: parent.width - playSlot.width - Theme.space.md
                text: best.subtitle !== "" ? qsTr("%1 · %2").arg(page.kindLabel(best.kind)).arg(best.subtitle) : page.kindLabel(best.kind)
                color: Theme.textSecondary
            }
            Item {
                id: playSlot
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                width: Theme.search.bestPlay
                height: width
                LibraryPlayButton {
                    id: playBest
                    objectName: "searchBestPlay"
                    width: parent.width
                    height: parent.height
                    y: best.revealed ? 0 : Theme.entrance.pageShift
                    opacity: (best.revealed ? 1 : 0) * (enabled ? 1 : Theme.library.disabledOpacity)
                    iconSize: Theme.icon.lg
                    text: qsTr("Reproducir %1").arg(best.title)
                    iconSource: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                    enabled: Backend.Player.canControl
                    onClicked: Backend.Library.playBestSearchResult()
                    Behavior on y {
                        YAnimator { duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                    Behavior on opacity {
                        OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                }
            }
        }
        onClicked: page.openRow({ kind: kind, objectId: objectId, title: title, subtitle: subtitle })
    }
}
