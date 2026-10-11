import QtQuick
import QtCore as QtCore
import Velo.Backend as Backend
import "../theme"
import "../components"
import "../components/library"

Item {
    id: root
    signal nowPlayingRequested()
    signal addToPlaylistRequested(string uri)
    signal queueRequested()
    signal devicesRequested()
    // Emitted when the Ajustes page opens (S9/N2: it is a page of this view; hosts may react).
    signal settingsRequested()
    function openSettings(section) {
        navigate("settings", section || "", "")
        settingsRequested()
    }
    property bool mica: false
    // Q1: width taken by the docked queue panel at the right (set by Main.qml); pages narrow.
    property real queueInset: 0
    // T5: «Reproduciendo» side panel, in the queue's column. Opening the queue REPLACES it (as in
    // Spotify): while queueInset > 0 the panel is hidden and reserves nothing. Wanted by default,
    // remembered between runs (QtCore.Settings, a local convenience, like the sidebar width).
    // With a narrow window it hides itself (it never squeezes the page under Theme.resize.minPage).
    // Depends only on the window width and the queue, never on the sidebar's own width (no loop).
    property alias sidePanelWanted: panelStore.nowPlayingPanel
    QtCore.Settings {
        id: panelStore
        category: "layout"
        property bool nowPlayingPanel: true
    }
    readonly property bool sidePanelFits: width - queueInset - Theme.size.sidebarMin - Theme.resize.minPage
                                          >= Theme.sidePanel.width + Theme.space.gutter
    readonly property bool sidePanelShown: sidePanelWanted && Backend.Player.hasSession && sidePanelFits
                                           && queueInset === 0 && !nowPlayingVisible
    readonly property real sidePanelInset: sidePanelShown ? Theme.sidePanel.width : 0
    // Total width taken at the right: the docked queue or the panel (never both).
    readonly property real rightInset: queueInset + sidePanelInset
    readonly property alias sidePanel: sidePanel
    // Height of everything stuck to the bottom (player bar + «Reproduciendo en…» strip); Main docks the queue and the device picker above it.
    readonly property real bottomReserve: Theme.size.playerBar + remoteStrip.reserve
    readonly property alias remoteStrip: remoteStrip
    // G4: the sidebar is resizable by dragging its right edge (sidebarMin..sidebarMax, and never
    // so wide that the page drops under Theme.resize.minPage beside the docked queue). The width
    // is remembered at zoom 100 % (QtCore.Settings, a local convenience) and follows the pointer
    // while dragging, with no animation. Double click on the edge: default width.
    property real sidebarDragWidth: -1
    QtCore.Settings {
        id: layoutStore
        category: "layout"
        property real sidebarWidth: 0       // at zoom 1; 0 = default
    }
    // T8: nor so wide that the top bar's search box (beside Atrás/Adelante) drops under its
    // minimum width (Windows caption buttons at the minimum window width used to squeeze it to
    // a sliver). Depends only on the window width, never on the sidebar itself.
    readonly property real sidebarStripCap: settingsButton.x - 2 * Theme.searchBox.gap - Theme.searchBox.minWidth
                                            - navigation.width - Theme.space.gutter
    readonly property real sidebarCeiling: Math.max(Theme.size.sidebarMin,
        Math.min(Theme.size.sidebarMax, width - rightInset - Theme.resize.minPage, sidebarStripCap))
    readonly property real sidebarWidth: Math.round(Math.max(Theme.size.sidebarMin, Math.min(sidebarCeiling,
        sidebarDragWidth > 0 ? sidebarDragWidth
        : layoutStore.sidebarWidth > 0 ? Theme.px(layoutStore.sidebarWidth) : Theme.size.sidebar)))
    readonly property real titleInset: Theme.size.titleBar
    // Windows-style caption buttons sit at the top right of the title strip; keep clear of them.
    readonly property real chromeTrailing: Backend.Settings.windowControls === "windows" ? 3 * Theme.windowControls.buttonWidth : 0
    property var history: []
    property int historyIndex: -1
    property string pageKind: "home"
    property string itemId: ""
    property string itemOwner: ""
    property string searchQuery: ""
    // Main binds this to "Now Playing is on screen" (single source for Visualizer.setSurfaces).
    property alias nowPlayingVisible: player.nowPlayingVisible
    readonly property alias currentPageItem: pageLoader.item
    // The container can override the display-name fallback with authoritative ownership.
    property var playlistOwnershipResolver: function(id, owner) {
        return Backend.Account.displayName !== "" && owner === Backend.Account.displayName
    }
    readonly property bool ownPlaylist: pageKind === "playlist" && playlistOwnershipResolver(itemId, itemOwner)
    // One lazy instance per page family, never one per history entry. The number of families is
    // fixed; hidden pages neither paint nor take input. Data/models stay owned by Rust.
    property var pageCache: ({})
    property string loadedRouteKey: ""
    function pageFamily(kind) {
        return ["home", "search", "artist", "albums", "artists", "profile", "settings", "show"].indexOf(kind) >= 0 ? kind : "tracks"
    }
    function pageComponent(kind) {
        return kind === "home" ? homePage : kind === "search" ? searchPage
            : kind === "artist" ? artistPage : kind === "albums" ? albumsPage
            : kind === "artists" ? artistsPage : kind === "profile" ? profilePage
            : kind === "settings" ? settingsPage : kind === "show" ? showPage : trackPage
    }

    function navigate(kind, id, owner) {
        const entry = { kind: kind, id: id || "", owner: owner || "", query: kind === "search" ? searchQuery : "", scrollY: 0 }
        if (historyIndex >= 0 && pageKind === kind && itemId === entry.id) return
        rememberScroll()
        const next = history.slice(0, historyIndex + 1)
        next.push(entry)
        history = next
        historyIndex = next.length - 1
        activate(entry, true)
    }
    // `rise`: a new route enters rising 8 px; Atrás/Adelante only fade (C1).
    function activate(entry, rise) {
        pendingScroll = -1
        restoreTarget = null
        if (pageLoader.item) { pageLoader.item.visible = false; pageLoader.item.enabled = false }
        pageKind = entry.kind
        itemId = entry.id
        itemOwner = entry.owner
        searchQuery = entry.query
        // Atrás / Adelante to a results page brings its query back into the top bar's field.
        if (entry.kind === "search" && entry.query && searchBox.text !== entry.query) searchBox.text = entry.query
        const routeKey = entry.kind + ":" + entry.id + ":" + (entry.query || "")
        // Settings leaves the backend's current list intact. Returning from it reuses those
        // rows too. Across distinct data routes, Rust must restore its authoritative models.
        if (entry.kind !== "settings" && routeKey !== loadedRouteKey) switch (entry.kind) {
        case "home": Backend.Library.loadHome(); break
        case "liked": Backend.Library.openLiked(); break
        case "playlist": Backend.Library.openPlaylist(entry.id); break
        case "album": Backend.Library.openAlbum(entry.id); break
        case "artist": Backend.Library.openArtist(entry.id); break
        case "albums": Backend.Library.openSavedAlbums(); break
        case "artists": Backend.Library.openFollowedArtists(); break
        case "profile": Backend.Library.openProfile(entry.id || "me"); break   // U4 ("me" = own)
        case "search": if (entry.query) Backend.Library.search(entry.query); break
        case "show": Backend.Library.openShow(entry.id); break
        case "settings": break   // S9/N2: local page, nothing to ask the backend for
        }
        if (entry.kind !== "settings") loadedRouteKey = routeKey
        const family = pageFamily(entry.kind)
        let page = pageCache[family]
        const reused = !!page
        if (!page) {
            page = pageComponent(entry.kind).createObject(pageLoader, {
                visible: false, enabled: false,
                width: Qt.binding(() => pageLoader.width), height: Qt.binding(() => pageLoader.height)
            })
            pageCache[family] = page
        }
        if (!page) return
        if (family === "tracks") { page.playlistId = itemId; page.ownPlaylist = Qt.binding(() => root.ownPlaylist) }
        else if (family === "profile") page.userId = itemId
        else if (family === "search") page.query = searchQuery
        else if (family === "artist") page.artistId = itemId
        else if (family === "show") page.showId = itemId
        else if (family === "settings") page.focusSection = itemId
        if (reused && family === "settings") Backend.Settings.refreshCacheSize()
        if (entry.localState) {
            for (const key of Object.keys(entry.localState)) page[key] = entry.localState[key]
        }
        if (entry.kind === "search" && entry.searchType) Backend.Library.setSearchType(entry.searchType)
        pageLoader.item = page
        page.enabled = true; page.visible = true
        enterPage(rise === true)
        // Atrás / Adelante: back to where the page was scrolled (P2-5), once its rows are there.
        pendingScroll = rise && entry.kind === "settings" && entry.id !== "" ? -1 : entry.scrollY || 0
        Qt.callLater(restoreScroll)
    }
    // Scroll position per history entry. The page's main scroller is its tallest Flickable
    // (ListView / GridView / Flickable); found on demand, nothing runs while scrolling.
    property real pendingScroll: -1
    property Flickable restoreTarget: null
    function mainFlickable(item) {
        let best = null
        const visit = it => {
            if (!it || !it.visible) return
            if (it instanceof Flickable && it.height > 0 && (!best || it.height > best.height)) best = it
            for (let i = 0; i < it.children.length; ++i) visit(it.children[i])
        }
        visit(item)
        return best
    }
    function rememberScroll() {
        if (historyIndex < 0 || historyIndex >= history.length) return
        const flick = mainFlickable(pageLoader.item)
        history[historyIndex].scrollY = flick ? flick.contentY - flick.originY : 0
        const state = {}
        for (const key of ["popularExpanded", "bioExpanded", "searchOpen"]) {
            if (pageLoader.item && key in pageLoader.item) state[key] = pageLoader.item[key]
        }
        history[historyIndex].localState = state
        if (pageKind === "search") history[historyIndex].searchType = Backend.Library.searchType
    }
    function restoreScroll() {
        if (pendingScroll < 0) { restoreTarget = null; return }
        const flick = restoreTarget || mainFlickable(pageLoader.item)
        if (!flick) return
        restoreTarget = flick
        if (Backend.Library.loading || (pageKind === "search" && Backend.Library.searchLoading)) return
        const room = flick.contentHeight - flick.height
        flick.contentY = flick.originY + Math.max(0, Math.min(pendingScroll, room))
        if (room >= pendingScroll) { pendingScroll = -1; restoreTarget = null }
    }
    Connections {
        target: root.pendingScroll >= 0 ? root.restoreTarget : null
        function onContentHeightChanged() { root.restoreScroll() }
        // The user scrolls first: their position wins.
        function onMovingChanged() { if (root.restoreTarget && root.restoreTarget.moving) root.pendingScroll = -1 }
    }
    Connections {
        target: Backend.Library
        function onLoadingChanged() { if (!Backend.Library.loading) Qt.callLater(root.restoreScroll) }
        function onSearchLoadingChanged() { if (!Backend.Library.searchLoading) Qt.callLater(root.restoreScroll) }
    }
    // T3: Ctrl+F / Ctrl+L (Main) and «Buscar» of the sidebar focus the top bar's field (its text
    // selected) without leaving the page; the results page opens on Enter.
    function focusSearch() { searchBox.focusField() }
    function showSearchResults(query) {
        const q = query.trim()
        if (q === "") return
        if (pageKind === "search" && pageLoader.item) { pageLoader.item.submit(q); return }
        searchQuery = q
        navigate("search", "", "")
    }
    // Motion (S1-2, C1): the outgoing page is hidden at once; the new
    // one fades in (and rises for a new route), `fast`, ease-out, on render-thread Animators. It
    // never blocks: the page is live from the first frame, and another navigation mid-way starts
    // the next page from the opacity reached, not from 0.
    function enterPage(rise) {
        const midway = pageIn.running
        pageIn.stop()                       // writes the reached opacity back
        if (Theme.motion.fast <= 0) { pageLoader.opacity = 1; pageLoader.y = 0; return }
        pageFade.from = midway ? pageLoader.opacity : 0
        pageRise.from = midway ? pageLoader.y : rise ? Theme.entrance.pageShift : 0
        pageIn.start()
    }
    ParallelAnimation {
        id: pageIn
        OpacityAnimator { id: pageFade; target: pageLoader; to: 1; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        YAnimator { id: pageRise; target: pageLoader; to: 0; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    // M1b: the open playlist was deleted / removed from the library (any menu): back to Inicio,
    // and its entries leave the history so Atrás never reopens it.
    function forgetPlaylist(id) {
        const kept = []
        let index = historyIndex
        for (let i = 0; i < history.length; ++i) {
            if (history[i].kind === "playlist" && history[i].id === id) { if (i <= historyIndex) --index }
            else kept.push(history[i])
        }
        history = kept
        historyIndex = Math.min(Math.max(index, -1), kept.length - 1)
        if (pageKind === "playlist" && itemId === id) navigate("home", "", "")
    }
    Connections {
        target: Backend.Actions
        function onSavedChanged(uri, saved) {
            if (!saved && uri.startsWith("spotify:playlist:")) root.forgetPlaylist(uri.slice("spotify:playlist:".length))
        }
    }
    function goBack() { if (historyIndex > 0) { rememberScroll(); activate(history[--historyIndex], false) } }
    function goForward() { if (historyIndex + 1 < history.length) { rememberScroll(); activate(history[++historyIndex], false) } }
    function rememberQuery(query) {
        searchQuery = query
        if (searchBox.text !== query) searchBox.text = query
        if (historyIndex >= 0) history[historyIndex].query = query
        if (pageKind === "search") loadedRouteKey = "search:" + itemId + ":" + query
    }

    Rectangle { width: root.sidebarWidth; height: parent.height; color: Theme.surface1; opacity: root.mica ? Theme.library.sidebarMicaOpacity : 1 }
    Rectangle { x: root.sidebarWidth; width: parent.width - x; height: parent.height; color: Theme.bg }
    // «Teñir la interfaz» (Settings.tintUi): a faint wash of the cover's hue over the ground,
    // under every page and panel. Flat, no animation; it changes with the track's palette.
    Rectangle {
        anchors.fill: parent
        visible: Backend.Settings.tintUi && Backend.Player.hasSession && !Theme.highContrast
        color: Qt.rgba(Theme.cover.base.r, Theme.cover.base.g, Theme.cover.base.b, Theme.tint.alpha)
    }
    // Mouse side buttons (XButton1 / XButton2): Atrás / Adelante, like Alt+← / Alt+→. Under
    // everything: it only accepts those two buttons, so clicks and wheel reach the pages, and
    // `veloNoDrag: false` keeps the title strip draggable over it.
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.BackButton | Qt.ForwardButton
        property bool veloNoDrag: false
        z: 3
        onClicked: mouse => mouse.button === Qt.BackButton ? root.goBack() : root.goForward()
    }
    LibrarySidebar {
        objectName: "librarySidebar"
        x: Theme.space.md
        y: root.titleInset + Theme.space.sm
        width: root.sidebarWidth - 2 * Theme.space.md
        height: player.y - y - Theme.space.md
        // G4: dragged narrow, the list switches to compact rows on its own.
        narrow: root.sidebarWidth < Theme.size.sidebarCompactBelow
        currentPage: root.pageKind
        currentId: root.itemId
        onNavigate: (kind, itemId, owner) => root.navigate(kind, itemId, owner)
        onSettingsRequested: root.openSettings("")
        onSearchRequested: root.focusSearch()
    }
    // G4: the sidebar's right edge; double click = default width.
    ResizeHandle {
        objectName: "sidebarResizeHandle"
        x: root.sidebarWidth - width / 2
        y: root.titleInset
        height: player.y - y
        z: 1
        growsRight: true
        value: root.sidebarWidth
        minimum: Theme.size.sidebarMin
        maximum: root.sidebarCeiling
        onResized: w => root.sidebarDragWidth = w
        onCommitted: w => { layoutStore.sidebarWidth = w / Theme.zoomFactor; root.sidebarDragWidth = -1 }
        onResetRequested: { layoutStore.sidebarWidth = 0; root.sidebarDragWidth = -1 }
    }
    Row {
        id: navigation
        x: root.sidebarWidth + Theme.space.gutter
        // Inside the title strip, like the official app (buttons there stay clickable: Controls).
        y: Math.round((root.titleInset - height) / 2)
        spacing: Theme.space.sm
        IconButton { text: qsTr("Atrás"); iconSource: Qt.resolvedUrl("../icons/arrow_back.svg"); enabled: root.historyIndex > 0; onClicked: root.goBack() }
        IconButton { text: qsTr("Adelante"); iconSource: Qt.resolvedUrl("../icons/arrow_back.svg"); rotation: 180; enabled: root.historyIndex + 1 < root.history.length; onClicked: root.goForward() }
    }
    LibraryLabel {
        id: crumb
        objectName: "libraryCrumb"
        anchors.left: navigation.right
        anchors.leftMargin: Theme.space.md
        anchors.verticalCenter: navigation.verticalCenter
        // T3: steps aside when the search box would otherwise drop under its minimum width.
        visible: root.crumbFits
        text: qsTr("Biblioteca")
        font.pixelSize: Theme.type.caption
        color: Theme.textSecondary
    }
    // T3: the search box, centered in the title strip like Spotify, between the
    // crumb (or Atrás/Adelante) and the buttons at the right. It shrinks to fit and never overlaps
    // them; the strip stays draggable around it (a text field takes clicks, TitleBar.qml).
    readonly property real searchLeftWithCrumb: crumb.x + crumb.implicitWidth + Theme.searchBox.gap
    readonly property real searchLeftBare: navigation.x + navigation.width + Theme.searchBox.gap
    // T8: the window's sleep-timer indicator (TitleBar) sits just left of Ajustes; Main tells its
    // width here and the box keeps clear of it as of the buttons.
    property real titleTimerWidth: 0
    readonly property real titleTrailingReserve: width - settingsButton.x + Theme.space.md
    readonly property real searchRight: settingsButton.x - Theme.searchBox.gap
                                        - (titleTimerWidth > 0 ? titleTimerWidth + Theme.space.md : 0)
    readonly property bool crumbFits: searchRight - searchLeftWithCrumb >= Theme.searchBox.minWidth
    SearchBox {
        id: searchBox
        objectName: "librarySearchBox"
        readonly property real minX: root.crumbFits ? root.searchLeftWithCrumb : root.searchLeftBare
        width: Math.max(Theme.size.iconButton, Math.min(Theme.searchBox.maxWidth, root.searchRight - minX))
        height: Theme.searchBox.height
        // Centered over the page column (right of the sidebar; the docked queue does not move it),
        // pushed aside only as far as the neighbours need.
        x: Math.round(Math.max(minX, Math.min((root.sidebarWidth + root.width - width) / 2, root.searchRight - width)))
        y: Math.round((root.titleInset - height) / 2)
        onSubmitted: query => root.showSearchResults(query)
        onNavigate: (kind, itemId, owner) => root.navigate(kind, itemId, owner)
    }
    readonly property alias searchBox: searchBox
    IconButton {
        id: themeButton
        anchors.right: parent.right
        // In the title strip, above the docked queue panel: only the caption buttons push it.
        anchors.rightMargin: Theme.space.gutter + root.chromeTrailing
        y: navigation.y
        text: Theme.dark ? qsTr("Usar tema claro") : qsTr("Usar tema oscuro")
        iconSource: Theme.dark ? Qt.resolvedUrl("../icons/light_mode.svg") : Qt.resolvedUrl("../icons/dark_mode.svg")
        // Main.qml photographs the window and crossfades to the new theme (A1).
        onClicked: Backend.Settings.theme = Theme.dark ? "light" : "dark"
    }
    // No hay settings.svg de Material Symbols Rounded en el repo: tune.svg es el más cercano.
    IconButton {
        id: settingsButton
        objectName: "openSettingsButton"
        anchors.right: themeButton.left; anchors.rightMargin: Theme.space.sm
        y: navigation.y; text: qsTr("Ajustes")
        iconSource: Qt.resolvedUrl("../icons/tune.svg")
        active: root.pageKind === "settings"
        toggle: true
        onClicked: root.openSettings("")
    }
    Loader {
        id: audioNotice
        objectName: "audioSetupNotice"
        active: root.noticeOn
        visible: active
        x: root.sidebarWidth + Theme.space.gutter
        y: root.titleInset + Theme.space.sm
        width: Math.max(0, root.width - x - Theme.space.gutter - root.rightInset)
        height: item ? item.implicitHeight : 0
        sourceComponent: AudioSetupNotice { width: audioNotice.width }
    }
    // Motion (G2, A-LB1): when audio gets activated the notice fades out (`fast`, ease-in) while
    // its space is still reserved; then the space goes at once (one relayout) and the page,
    // pushed back down by a transform, glides up (`base`, ease-out). Only opacity and a
    // Translate; instant when the motion tokens are 0. Coming back mid-way restores everything.
    readonly property bool noticeWanted: Backend.Account.signedIn && !Backend.Account.audioReady
    property bool noticeOn: false
    onNoticeWantedChanged: {
        if (noticeWanted) {
            noticeExit.stop(); noticeRise.stop()
            noticeShift.y = 0
            audioNotice.opacity = 1
            noticeOn = true
        } else if (noticeOn && audioNotice.item && Theme.motion.fast > 0) {
            noticeGap = audioNotice.height + Theme.space.xl
            noticeExit.restart()
        } else {
            noticeOn = false
        }
    }
    property real noticeGap: 0
    SequentialAnimation {
        id: noticeExit
        OpacityAnimator { target: audioNotice; from: 1; to: 0; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeIn }
        ScriptAction {
            script: {
                root.noticeOn = false
                audioNotice.opacity = 1
                if (Theme.motion.base > 0) { noticeShift.y = root.noticeGap; noticeRise.restart() }
            }
        }
    }
    NumberAnimation { id: noticeRise; target: noticeShift; property: "y"; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    // The page area. The Loader sits inside unanchored so its y can rise on entry (YAnimator)
    // without fighting anchors. The page edge follows the docked queue at once: easing it would
    // animate a width (relayout), so the queue panel moves instead.
    Item {
        id: pageArea
        anchors.left: parent.left
        anchors.leftMargin: root.sidebarWidth + Theme.space.gutter
        anchors.right: parent.right
        anchors.rightMargin: Theme.space.gutter + root.rightInset
        anchors.top: parent.top
        anchors.topMargin: root.titleInset + Theme.space.sm + (root.noticeOn ? audioNotice.height + Theme.space.xl : 0)
        anchors.bottom: player.top
        anchors.bottomMargin: Theme.space.xl
        Item {
            id: pageLoader
            property Item item: null
            transform: Translate { id: noticeShift }
            width: parent.width
            height: parent.height
        }
    }
    // Contrato para el trabajador del perfil: ruta profile/me, sin abrir tracks anteriores.
    Component { id: homePage; HomePage { onNavigate: (kind, itemId, owner) => root.navigate(kind, itemId, owner); onQueueRequested: root.queueRequested() } }
    Component { id: trackPage; TrackListPage { onAddToPlaylistRequested: uri => root.addToPlaylistRequested(uri); onProfileRequested: userId => root.navigate("profile", userId, "") } }
    Component { id: profilePage; ProfilePage { onNavigate: (kind, itemId, owner) => root.navigate(kind, itemId, owner) } }
    Component { id: searchPage; SearchPage { onQuerySubmitted: query => root.rememberQuery(query); onAddToPlaylistRequested: uri => root.addToPlaylistRequested(uri); onNavigate: (kind, itemId, owner) => root.navigate(kind, itemId, owner) } }
    Component { id: artistPage; ArtistPage { onAlbumRequested: albumId => root.navigate("album", albumId, ""); onArtistRequested: id => root.navigate("artist", id, "") } }
    Component { id: albumsPage; AlbumGridPage { onAlbumRequested: albumId => root.navigate("album", albumId, "") } }
    Component { id: artistsPage; ArtistGridPage { onArtistRequested: artistId => root.navigate("artist", artistId, "") } }
    // S9/N2: Ajustes as a library page; the route's id names the section to bring into view.
    Component { id: showPage; PodcastPage {} }
    Component { id: settingsPage; SettingsPage {} }
    NowPlayingSidePanel {
        id: sidePanel
        shown: root.sidePanelShown
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: player.top
        anchors.topMargin: Theme.size.titleBar + Theme.queue.inset
        anchors.bottomMargin: Theme.queue.inset
        width: Theme.sidePanel.width
        onQueueRequested: root.queueRequested()
    }
    // T3: while another device plays, an accent strip sits under the bar (the bar rises by its height).
    RemoteDeviceStrip {
        id: remoteStrip
        anchors.bottom: parent.bottom
        width: parent.width
    }
    LibraryPlayerBar {
        id: player
        sidePanelActive: root.sidePanelWanted
        onSidePanelToggled: root.sidePanelWanted = !root.sidePanelWanted
        anchors.bottom: remoteStrip.top
        width: parent.width
        height: Theme.size.playerBar
        onNowPlayingRequested: root.nowPlayingRequested()
        onQueueRequested: root.queueRequested()
        onDevicesRequested: root.devicesRequested()
    }
    // E6: "Ver créditos" from the song menus of this view (Nav.showCredits).
    CreditsDialog { id: creditsDialog }
    readonly property alias creditsDialog: creditsDialog
    Shortcut { sequence: "Alt+Left"; enabled: root.visible; onActivated: root.goBack() }
    Shortcut { sequence: "Alt+Right"; enabled: root.visible; onActivated: root.goForward() }
    Component.onCompleted: { noticeOn = noticeWanted; navigate("home", "", "") }
}
