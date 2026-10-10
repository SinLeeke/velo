import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import ".."

Item {
    id: page
    property string playlistId: ""
    property bool ownPlaylist: false
    property bool searchResults: false
    property alias listView: tracks
    readonly property alias sortMenu: sortMenu
    signal addToPlaylistRequested(string uri)
    // U4: the owner's name in a playlist header opens their profile (Library.currentOwnerId).
    signal profileRequested(string userId)
    readonly property bool ownerLinked: kind === "playlist" && !searchResults
        && Backend.Library.currentOwnerId !== "" && Backend.Library.currentSubtitle !== ""
    // R4: the route that opened this page ("playlist", "album", "liked"; Library.qml passes
    // root.pageKind). Until it does, the kind Rust reports stands in for it.
    property string routeKind: ""
    readonly property string listKind: routeKind !== "" ? routeKind : kind
    readonly property string kindLabel: listKind === "album" ? qsTr("Álbum") : listKind === "playlist" || listKind === "liked" ? qsTr("Playlist") : ""
    // Spotify URI of the open list, for its "…" menu and the save button ("" for «Tus me gusta»).
    readonly property string listUri: playlistId === "" ? ""
        : listKind === "playlist" ? "spotify:playlist:" + playlistId
        : listKind === "album" ? "spotify:album:" + playlistId : ""
    // Album / someone else's playlist in the library: -1 asking Spotify, 0 no, 1 yes (kept here
    // so a card menu inspecting another URI through Actions does not flip the button).
    property int saved: -1
    property bool initialized: false
    readonly property bool canSave: listUri !== "" && !ownPlaylist && !searchResults
    function refreshSaved() {
        saved = -1
        if (canSave) Backend.Actions.inspectSaved(listUri)
    }
    onListUriChanged: if (initialized) refreshSaved()
    Component.onCompleted: { initialized = true; refreshSaved() }
    Connections {
        target: Backend.Actions
        function onInspectedSavedChanged() {
            if (Backend.Actions.inspectedUri === page.listUri && Backend.Actions.inspectedSaved >= 0)
                page.saved = Backend.Actions.inspectedSaved
        }
        function onSavedChanged(uri, saved) { if (uri === page.listUri) page.saved = saved ? 1 : 0 }
    }
    // R4 (P1-1): a failed load is not an empty list. Library.loadError carries Rust's message;
    // "Reintentar" asks for the next page again, or reopens the list when nothing came.
    property string localLoadError: ""
    readonly property string loadError: searchResults ? Backend.Library.searchError
        : ("loadError" in Backend.Library && Backend.Library.loadError !== "" ? Backend.Library.loadError : localLoadError)
    readonly property bool failed: loadError !== "" && !Backend.Library.loading
    readonly property bool firstLoad: Backend.Library.loading && tracks.count === 0
    function reload() {
        localLoadError = ""
        if (tracks.count > 0) { requestedAtCount = tracks.count; Backend.Library.fetchMore(); return }
        requestedAtCount = -1
        if (listKind === "liked") Backend.Library.openLiked()
        else if (listKind === "album" && playlistId !== "") Backend.Library.openAlbum(playlistId)
        else if (listKind === "playlist" && playlistId !== "") Backend.Library.openPlaylist(playlistId)
        else if (searchResults && Backend.Library.searchQuery !== "") Backend.Library.search(Backend.Library.searchQuery)
    }
    // R4 (P1-6): one song menu for every row of this list (a row fills it when it opens it).
    TrackMenu { id: sharedTrackMenu }
    readonly property alias trackMenu: sharedTrackMenu
    Loader {
        id: listMenu
        active: false
        sourceComponent: MediaMenu { uri: page.listUri; owned: page.ownPlaylist; mediaTitle: Backend.Library.currentTitle }
    }
    function openListMenu(anchor) {
        listMenu.active = true
        const menu = listMenu.item as MediaMenu
        menu.popup(anchor, 0, anchor.height)
    }
    property int requestedAtCount: -1
    // "#" column: one width for every row, measured here (not per delegate) for the digits of
    // the longest index of this list, in tabular figures, so titles align across the rows. It
    // changes only when the count crosses a power of ten (U4: a fixed 5 digits pushed the rows
    // right of the header art).
    readonly property int numberDigits: String(Math.max(1, tracks.count, Backend.Library.currentTrackCount)).length
    readonly property string numberSample: "0".repeat(numberDigits)
    readonly property real numberWidth: Math.ceil(numberMetrics.advanceWidth)
    // F3 columns. Widths are derived once per page width (window resize, queue docked), never
    // per frame; a hidden column is width 0. Fecha hides first, then Álbum.
    readonly property string kind: Backend.Library.currentKind
    readonly property bool datedList: kind === "playlist" || kind === "liked"
    // F4 / S9: "Ver como: Compacta". Playlists and "Tus me gusta": Library.compactView (starts from
    // Settings.compactTracks; Library.setCompact keeps a per-list override for the session).
    // Other track lists (album, results) follow Settings.compactTracks.
    readonly property bool compact: datedList ? Backend.Library.compactView : Backend.Settings.compactTracks === true
    // F4: list tools (search in the list, "Clasificar por", "Ver como").
    readonly property bool listTools: datedList && Backend.Library.currentListable && !searchResults
    property bool searchOpen: false
    readonly property real rowWidth: tracks.width - Theme.space.sm
    readonly property real artWidth: compact ? 0 : Theme.trackList.rowArt
    readonly property bool showAlbum: kind !== "album" && rowWidth >= Theme.trackList.albumMinRowWidth
    readonly property bool showAdded: datedList && rowWidth >= Theme.trackList.addedMinRowWidth
    readonly property real addedWidth: showAdded ? Theme.trackList.addedWidth : 0
    readonly property real flexWidth: rowWidth - 2 * Theme.trackList.rowInset - numberWidth
        - (compact ? 0 : artWidth + Theme.space.md) - (showAdded ? addedWidth + Theme.space.md : 0)
        - Theme.trackList.trailingWidth - 4 * Theme.space.md
    readonly property real albumWidth: showAlbum ? Math.round(flexWidth * Theme.trackList.albumShare) : 0
    readonly property real titleWidth: Math.max(0, flexWidth - (showAlbum ? albumWidth + Theme.space.md : 0))
    // F3 header line: "Juan · 48 canciones, 2 h 31 min" ("más de …" while pages are missing).
    function durationText(ms) {
        const hours = Math.floor(ms / 3600000), minutes = Math.floor(ms % 3600000 / 60000)
        return hours > 0 ? qsTr("%1 h %2 min").arg(hours).arg(minutes) : qsTr("%1 min").arg(minutes)
    }
    readonly property string summary: {
        const subtitle = ownerLinked ? "" : Backend.Library.currentSubtitle
        if (!datedList) return subtitle
        const parts = subtitle !== "" ? [subtitle] : []
        const count = Backend.Library.currentTrackCount
        if (count > 0) {
            let text = count === 1 ? qsTr("1 canción") : qsTr("%1 canciones").arg(Number(count).toLocaleString(Qt.locale(), "f", 0))
            const ms = Backend.Library.currentDurationMs
            if (ms > 0)
                text = Backend.Library.currentDurationExact ? qsTr("%1, %2").arg(text).arg(durationText(ms))
                                                            : qsTr("%1, más de %2").arg(text).arg(durationText(ms))
            parts.push(text)
        }
        return parts.join(qsTr(" · "))
    }
    TextMetrics {
        id: numberMetrics
        text: page.numberSample
        font.family: Theme.family
        font.pixelSize: Theme.type.body
        font.features: Theme.tabularFeatures
    }

    function resetResults() {
        requestedAtCount = -1
        tracks.currentIndex = -1
        tracks.positionViewAtBeginning()
    }
    function fetchNearEnd() {
        if (visible && !failed && Backend.Library.currentListable && Backend.Library.hasMore && !Backend.Library.loading
                && tracks.contentY - tracks.originY + tracks.height >= tracks.contentHeight - Theme.size.trackRow * 4
                && requestedAtCount !== tracks.count) {
            requestedAtCount = tracks.count
            Backend.Library.fetchMore()
        }
    }
    SortMenu { id: sortMenu; objectName: "sortMenu" }
    function openSortMenu(anchor) { sortMenu.popup(anchor, anchor.width - sortMenu.width, anchor.height) }
    Connections {
        target: Backend.Library
        function onCurrentKindChanged() { page.localLoadError = ""; page.resetResults() }
        // A new view of the list: show it from the top.
        function onSortKeyChanged() { page.resetResults() }
        function onSortDescendingChanged() { page.resetResults() }
        function onLoadingChanged() {
            if (Backend.Library.loading) page.localLoadError = ""
            else Qt.callLater(page.fetchNearEnd)
        }
    }
    Connections {
        target: Backend.Player
        function onError(message) {
            if (page.visible && Backend.Library.loading && !page.searchResults) page.localLoadError = message
        }
    }
    ListView {
        id: tracks
        objectName: "libraryTracks"
        anchors.fill: parent
        clip: true
        reuseItems: true
        model: Backend.Library.currentListable ? Backend.Library.tracks : null
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        // C7: a removed song fades out (fast, easeIn) and the rows below close the gap (y of the
        // delegates, fast, easeOut; no relayout). Only granular model changes animate; a reload
        // does not. NumberAnimation, not Y/OpacityAnimator: in view transitions the Animators
        // did not pick up the transition's from/to (checked in a frame-by-frame probe).
        // TrackRow resets its opacity when the pooled delegate is reused.
        remove: Transition {
            NumberAnimation { property: "opacity"; to: 0; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeIn }
        }
        displaced: Transition {
            NumberAnimation { property: "y"; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        onContentYChanged: page.fetchNearEnd()
        onCountChanged: Qt.callLater(page.fetchNearEnd)
        onHeightChanged: Qt.callLater(page.fetchNearEnd)
        keyNavigationEnabled: true
        header: Column {
            width: tracks.width - Theme.space.sm
            spacing: Theme.space.xl
            LibraryPageHeader {
                visible: !page.searchResults
                width: parent.width
                kindLabel: page.kindLabel
                title: Backend.Library.currentTitle
                titleObjectName: "listTitle"
                artSource: Backend.Library.currentImageUrl
                likedArt: page.listKind === "liked"
                // "Juan · 48 canciones, 2 h 31 min": the owner is a link to the profile,
                // underlined on hover / keyboard focus, as in Spotify.
                meta: [
                    Row {
                        width: parent ? parent.width : 0
                        LibraryLabel {
                            id: ownerLink
                            objectName: "listOwnerLink"
                            visible: page.ownerLinked
                            width: Math.min(implicitWidth, parent.width / 2)
                            text: Backend.Library.currentSubtitle
                            font.weight: Theme.type.semibold
                            font.underline: ownerArea.containsMouse || activeFocus
                            activeFocusOnTab: visible
                            Accessible.role: Accessible.Link
                            Accessible.name: qsTr("Ver el perfil de %1").arg(text)
                            Accessible.onPressAction: page.profileRequested(Backend.Library.currentOwnerId)
                            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
                            Keys.onReturnPressed: page.profileRequested(Backend.Library.currentOwnerId)
                            Keys.onSpacePressed: page.profileRequested(Backend.Library.currentOwnerId)
                            // P3-7: a visible focus ring around the link, not only the underline.
                            Rectangle {
                                anchors.fill: parent
                                anchors.margins: -Theme.space.xxs
                                visible: ownerLink.activeFocus
                                radius: Theme.radius.xs
                                color: Theme.library.clearPaint
                                border.width: Theme.stroke.focus
                                border.color: Theme.focusRing
                            }
                            MouseArea {
                                id: ownerArea
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: page.profileRequested(Backend.Library.currentOwnerId)
                            }
                        }
                        LibraryLabel {
                            objectName: "listSummary"
                            width: parent.width - (ownerLink.visible ? ownerLink.width : 0)
                            text: page.ownerLinked && page.summary !== "" ? qsTr(" · %1").arg(page.summary) : page.summary
                            color: Theme.textSecondary
                            wrapMode: Text.Wrap
                        }
                    }
                ]
            }
            Item {
                id: actions
                width: parent.width
                // Very narrow page (minimum window with the queue docked): tools drop to a
                // second line instead of covering the play buttons.
                readonly property real buttonRun: 2 * Theme.library.cardWidth + Theme.space.lg
                    + (page.canSave ? Theme.size.titleBar + Theme.space.lg : 0)
                    + (page.listUri !== "" && !page.searchResults ? Theme.size.titleBar + Theme.space.lg : 0)
                readonly property bool stacked: page.listTools
                    && width < buttonRun + Theme.space.lg + 2 * Theme.size.iconButton + tools.spacing
                height: playButtons.height + (stacked ? Theme.size.titleBar + Theme.space.sm : 0)
                // Below this width the sort button keeps only its icon.
                readonly property bool roomy: width >= Theme.trackList.toolbarRoomyWidth
                Flow {
                    id: playButtons
                    width: Math.min(actions.buttonRun, parent.width)
                    spacing: Theme.space.lg
                    LibraryButton {
                        objectName: "playCurrentButton"
                        width: Theme.library.cardWidth
                        primary: true
                        text: qsTr("Reproducir")
                        iconSource: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                        enabled: Backend.Player.canControl
                        // v0.9.2: always in order; «Aleatorio» next to it plays shuffled.
                        onClicked: Backend.Library.playCurrent()
                    }
                    // v0.9.2 (user: "un botón de reproducir aleatorio además del normal"): replaces
                    // the M2 shuffle switch. Every press loads the list again with shuffle on, so
                    // the engine draws a new order each time.
                    LibraryButton {
                        objectName: "shufflePlayButton"
                        width: Theme.library.cardWidth
                        text: qsTr("Aleatorio")
                        iconSource: Qt.resolvedUrl("../../icons/shuffle.svg")
                        enabled: Backend.Player.canControl
                        onClicked: Backend.Library.shuffleCurrent()
                    }
                    // R4 (P2-2): save the album / someone else's playlist, and the list's own menu
                    // (queue, radio, link), as on the artist page.
                    IconButton {
                        objectName: "listSaveButton"
                        visible: page.canSave
                        width: Theme.size.titleBar
                        height: Theme.size.titleBar
                        text: page.saved === 1 ? qsTr("Quitar de tu biblioteca") : qsTr("Guardar en tu biblioteca")
                        iconSource: page.saved === 1 ? Qt.resolvedUrl("../../icons/check.svg") : Qt.resolvedUrl("../../icons/add.svg")
                        iconSize: Theme.icon.lg
                        ink: Theme.textSecondary
                        active: page.saved === 1
                        activeFill: hovered ? Theme.surface2 : Theme.library.clearPaint
                        activeInk: Theme.accent
                        Accessible.checkable: true
                        // Unknown state (still asking Spotify): disabled rather than guessing.
                        enabled: page.saved >= 0
                        onClicked: Backend.Actions.setSaved(page.listUri, page.saved !== 1)
                    }
                    IconButton {
                        id: listMore
                        objectName: "listMoreButton"
                        visible: page.listUri !== "" && !page.searchResults
                        width: Theme.size.titleBar
                        height: Theme.size.titleBar
                        text: qsTr("Más opciones de %1").arg(Backend.Library.currentTitle)
                        iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                        iconSize: Theme.icon.lg
                        ink: Theme.textSecondary
                        onClicked: page.openListMenu(listMore)
                    }
                }
                Row {
                    id: tools
                    visible: page.listTools
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: Theme.size.titleBar
                    spacing: Theme.space.sm
                    LibraryField {
                        id: filterField
                        objectName: "playlistFilterField"
                        visible: page.searchOpen || Backend.Library.filterQuery !== ""
                        width: Math.max(Theme.size.coverMin / 2, Math.min(Theme.trackList.searchFieldWidth,
                               actions.width - (actions.stacked ? 0 : playButtons.width + Theme.space.lg) - sortWide.width - tools.spacing))
                        leftPadding: Theme.space.md + Theme.icon.sm + Theme.space.sm
                        placeholderText: page.kind === "liked" ? qsTr("Buscar en Tus me gusta") : qsTr("Buscar en la playlist")
                        Component.onCompleted: text = Backend.Library.filterQuery
                        onTextEdited: Backend.Library.setFilter(text)
                        Keys.onEscapePressed: {
                            if (text !== "") { text = ""; Backend.Library.setFilter("") }
                            else { page.searchOpen = false; tracks.forceActiveFocus() }
                        }
                        onActiveFocusChanged: if (!activeFocus && text === "") page.searchOpen = false
                        Connections {
                            target: Backend.Library
                            // Remembered search of the list just opened (never while typing).
                            function onFilterQueryChanged() {
                                if (!filterField.activeFocus && filterField.text !== Backend.Library.filterQuery)
                                    filterField.text = Backend.Library.filterQuery
                            }
                        }
                        IconImage {
                            x: Theme.space.md
                            anchors.verticalCenter: parent.verticalCenter
                            source: Qt.resolvedUrl("../../icons/search.svg")
                            sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                            color: Theme.textSecondary
                        }
                    }
                    IconButton {
                        objectName: "playlistSearchButton"
                        anchors.verticalCenter: parent.verticalCenter
                        visible: !filterField.visible
                        text: page.kind === "liked" ? qsTr("Buscar en Tus me gusta") : qsTr("Buscar en la playlist")
                        iconSource: Qt.resolvedUrl("../../icons/search.svg")
                        onClicked: { page.searchOpen = true; filterField.forceActiveFocus() }
                    }
                    Item {
                        id: sortWide
                        width: actions.roomy ? sortButton.implicitWidth : sortIcon.width
                        height: tools.height
                        LibraryButton {
                            id: sortButton
                            objectName: "sortButton"
                            visible: actions.roomy
                            text: sortMenu.labelFor(Backend.Library.sortKey)
                            iconSource: Qt.resolvedUrl("../../icons/tune.svg")
                            Accessible.name: qsTr("Clasificar por: %1").arg(text)
                            onClicked: page.openSortMenu(sortButton)
                        }
                        IconButton {
                            id: sortIcon
                            anchors.verticalCenter: parent.verticalCenter
                            visible: !actions.roomy
                            text: qsTr("Clasificar por: %1").arg(sortMenu.labelFor(Backend.Library.sortKey))
                            iconSource: Qt.resolvedUrl("../../icons/tune.svg")
                            onClicked: page.openSortMenu(sortIcon)
                        }
                    }
                }
            }
            // F5: genre/mood chips of the open list (Library.trackTags, most frequent first).
            // No tags (loading, no data) → no row and no reserved space. A chip filters the
            // list (Library.filterTag); the active chip clears it. When the tags arrive the
            // chips fade in and rise a few px, staggered (fast, easeOut; opacity + Translate).
            ListView {
                id: tagChips
                objectName: "trackTagChips"
                readonly property bool wanted: page.datedList && !page.searchResults
                    && Backend.Library.currentListable && Backend.Library.trackTags.length > 0
                // Chips created within this window of the tags arriving play their entrance.
                property bool entering: false
                visible: wanted
                width: parent.width
                height: Theme.library.chipHeight
                orientation: ListView.Horizontal
                spacing: Theme.space.sm
                clip: true
                reuseItems: true
                boundsBehavior: Flickable.StopAtBounds
                model: wanted ? Backend.Library.trackTags : []
                onWantedChanged: {
                    entering = wanted && Theme.motion.fast > 0
                    if (entering) enterWindow.restart()
                }
                // One-shot window (no Timer): lasts as long as the last staggered chip's delay.
                PauseAnimation {
                    id: enterWindow
                    duration: Theme.tagChips.stagger * Theme.tagChips.staggerMax + Theme.motion.fast
                    onFinished: tagChips.entering = false
                }
                delegate: LibraryChip {
                    id: chip
                    required property string modelData
                    required property int index
                    text: modelData
                    selected: Backend.Library.activeTag === modelData
                    Accessible.name: qsTr("Filtrar por %1").arg(modelData)
                    onClicked: Backend.Library.filterTag(selected ? "" : modelData)
                    transform: Translate { id: chipRise }
                    function enter() {
                        chipEntrance.stop()
                        if (!tagChips.entering) { opacity = 1; chipRise.y = 0; return }
                        opacity = 0
                        chipRise.y = Theme.tagChips.rise
                        chipDelay.duration = Math.min(index, Theme.tagChips.staggerMax) * Theme.tagChips.stagger
                        chipEntrance.start()
                    }
                    Component.onCompleted: enter()
                    ListView.onReused: enter()
                    ListView.onPooled: { chipEntrance.stop(); opacity = 1; chipRise.y = 0 }
                    SequentialAnimation {
                        id: chipEntrance
                        PauseAnimation { id: chipDelay }
                        ParallelAnimation {
                            OpacityAnimator { target: chip; from: 0; to: 1; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                            NumberAnimation { target: chipRise; property: "y"; to: 0; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                        }
                    }
                }
            }
            // F3: column header strip, aligned with TrackRow's columns.
            Item {
                objectName: "trackColumns"
                visible: Backend.Library.currentListable
                width: parent.width
                height: Theme.trackList.headerRow
                Row {
                    x: Theme.trackList.rowInset
                    width: parent.width - 2 * Theme.trackList.rowInset
                    height: parent.height
                    spacing: Theme.space.md
                    LibraryLabel {
                        width: page.numberWidth
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("#")
                        color: Theme.textSecondary
                        font.pixelSize: Theme.type.caption
                        horizontalAlignment: Text.AlignRight
                        elide: Text.ElideNone
                    }
                    LibraryLabel {
                        width: page.titleWidth + (page.compact ? 0 : page.artWidth + Theme.space.md)
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Título")
                        color: Theme.textSecondary
                        font.pixelSize: Theme.type.caption
                    }
                    LibraryLabel {
                        width: page.albumWidth
                        visible: width > 0
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Álbum")
                        color: Theme.textSecondary
                        font.pixelSize: Theme.type.caption
                    }
                    LibraryLabel {
                        width: page.addedWidth
                        visible: width > 0
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Fecha en que se agregó")
                        color: Theme.textSecondary
                        font.pixelSize: Theme.type.caption
                    }
                    Item { width: Theme.icon.sm; height: parent.height }
                    Item {
                        width: Theme.trackList.durationWidth
                        height: parent.height
                        IconImage {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            source: Qt.resolvedUrl("../../icons/schedule.svg")
                            sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                            color: Theme.textSecondary
                            Accessible.role: Accessible.ColumnHeader
                            Accessible.name: qsTr("Duración")
                        }
                    }
                    Item { width: Theme.size.iconButton; height: parent.height }
                }
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: Theme.stroke.hairline
                    color: Theme.hairline
                }
            }
        }
        delegate: TrackRow {
            width: tracks.width - Theme.space.sm
            numberWidth: page.numberWidth
            titleWidth: page.titleWidth
            albumWidth: page.albumWidth
            addedWidth: page.addedWidth
            compact: page.compact
            playlistId: page.playlistId
            ownPlaylist: page.ownPlaylist
            sharedMenu: sharedTrackMenu
            onSelected: rowIndex => tracks.currentIndex = rowIndex
            onAddToPlaylistRequested: uri => page.addToPlaylistRequested(uri)
        }
        footer: Column {
            width: tracks.width
            topPadding: Theme.space.xl
            spacing: Theme.space.md
            // First load: a quiet skeleton (static plates, no shimmer: nothing runs while it
            // waits) instead of any "empty" text.
            Repeater {
                model: page.firstLoad && Backend.Library.currentListable ? Theme.library.skeletonRows : 0
                delegate: Row {
                    objectName: "listSkeletonRow"
                    x: Theme.trackList.rowInset
                    height: page.compact ? Theme.trackList.compactRow : Theme.size.trackRow
                    spacing: Theme.space.md
                    opacity: Theme.library.disabledOpacity
                    Rectangle { visible: !page.compact; anchors.verticalCenter: parent.verticalCenter; width: Theme.trackList.rowArt; height: width; radius: Theme.radius.sm; color: Theme.surface2 }
                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.space.sm
                        Rectangle { width: Theme.size.coverMin; height: Theme.space.md; radius: height / 2; color: Theme.surface2 }
                        Rectangle { visible: !page.compact; width: Theme.size.coverMin / 2; height: Theme.space.sm; radius: height / 2; color: Theme.surface2 }
                    }
                }
            }
            LibraryLabel {
                objectName: "listFooterStatus"
                width: parent.width
                visible: !page.firstLoad && (page.failed || Backend.Library.loading || Backend.Library.viewBusy
                         || (tracks.count === 0 && Backend.Library.currentListable))
                // F4: a search or sort first loads the rest of the list (viewBusy).
                text: page.failed ? page.loadError
                      : Backend.Library.viewBusy ? qsTr("Cargando toda la lista para buscar y ordenar…")
                      : Backend.Library.loading ? qsTr("Cargando más canciones…")
                      : page.searchResults ? qsTr("No hay resultados. Prueba con otro título o artista.")
                      : Backend.Library.filterQuery !== "" ? qsTr("No se encontró «%1» en esta lista.").arg(Backend.Library.filterQuery)
                      : Backend.Library.activeTag !== "" ? qsTr("No hay canciones de «%1» en esta lista.").arg(Backend.Library.activeTag)
                      : page.listKind === "liked" ? qsTr("Las canciones que guardes con el corazón aparecerán aquí.")
                      : qsTr("Esta lista todavía no tiene canciones.")
                color: page.failed ? Theme.text : Theme.textSecondary
                wrapMode: Text.Wrap
            }
            LibraryButton {
                objectName: "listRetryButton"
                visible: page.failed
                text: qsTr("Reintentar")
                onClicked: page.reload()
            }
            LibraryButton {
                visible: Backend.Library.hasMore && Backend.Library.currentListable && !Backend.Library.loading && !Backend.Library.viewBusy && !page.failed
                width: Theme.size.coverMin
                text: qsTr("Cargar más canciones")
                onClicked: { page.requestedAtCount = tracks.count; Backend.Library.fetchMore() }
            }
            Item { width: parent.width; height: Theme.space.xl }
        }
    }
}
