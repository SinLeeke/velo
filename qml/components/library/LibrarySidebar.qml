import QtQuick
import QtQml.Models
import QtQuick.Templates as T
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import "../motion"
import ".."

// S9/N3: the library sidebar as in Spotifast / the official app. Fixed Inicio / Buscar, then the
// «Biblioteca» header (search in the library, create playlist, compact view), the kind chips,
// the sort selector and one list (Library.sidebarItems: playlists, albums, artists, podcasts;
// pins first, then Settings.librarySort, both applied in Rust).
Item {
    id: sidebar
    property string currentPage: "home"
    property string currentId: ""
    signal navigate(string kind, string itemId, string owner)
    signal settingsRequested()
    signal searchRequested()
    Keys.onShortcutOverride: event => {
        const w = sidebar.Window.window
        if (event.key === Qt.Key_Space && w && w.activeFocusItem && w.activeFocusItem.activeFocusOnTab)
            event.accepted = true
    }
    property bool creating: false
    property bool searchOpen: false

    // G4: set by Library when the sidebar is dragged narrower than the art rows need; the list is
    // compact then whatever the user's choice (which comes back as soon as it is wide again).
    property bool narrow: false
    readonly property bool userCompact: Backend.Settings.sidebarCompact === true
    readonly property bool compact: narrow || userCompact
    readonly property string sortKey: Backend.Settings.librarySort
    function toggleCompact() { Backend.Settings.sidebarCompact = !userCompact }
    function setSort(key) { Backend.Settings.librarySort = key }
    function sortLabel(key) {
        return key === "recent_added" ? qsTr("Agregado recientemente")
             : key === "alphabetical" ? qsTr("Alfabético")
             : key === "creator" ? qsTr("Creador")
             : qsTr("Reproducido recientemente")
    }
    function subtitleFor(kind, creator) {
        switch (kind) {
        case "playlist": return creator !== "" ? qsTr("Playlist · %1").arg(creator) : qsTr("Playlist")
        case "album": return creator !== "" ? qsTr("Álbum · %1").arg(creator) : qsTr("Álbum")
        case "artist": return qsTr("Artista")
        case "show": return creator !== "" ? qsTr("Pódcast · %1").arg(creator) : qsTr("Pódcast")
        }
        return creator
    }
    function iconFor(kind) {
        return kind === "artist" ? Qt.resolvedUrl("../../icons/person.svg")
             : kind === "album" ? Qt.resolvedUrl("../../icons/library_music.svg")
             : kind === "show" ? Qt.resolvedUrl("../../icons/record_voice_over.svg")
             : Qt.resolvedUrl("../../icons/music_note.svg")
    }
    function open(kind, objectId, creator) {
        navigate(kind, objectId, kind === "playlist" ? creator : "")
    }
    // Playlist ids in the order shown (previews / tests).
    function playlistOrder() {
        const result = []
        for (let i = 0; i < entries.items.count; ++i) {
            const row = entries.items.get(i).model
            if (row.kind === "playlist") result.push(row.objectId)
        }
        return result
    }
    function create() {
        const name = playlistName.text.trim()
        if (!name) return
        Backend.Library.createPlaylist(name)
        playlistName.clear()
        creating = false
    }
    function closeSearch() {
        searchField.text = ""
        Backend.Library.searchLibrary("")
        searchOpen = false
    }

    // Row metrics: 48 px art in a 64 px row; compact rows are one line of text.
    readonly property int artSize: Theme.size.trackRow - Theme.space.sm
    readonly property int rowHeight: compact ? Theme.trackList.compactRow : Theme.size.trackRow + Theme.space.sm

    // S1: the list re-enters (fade + 4 px rise, `fast`, easeOut) when the chip, the sort or the
    // compact view changes, so the new content reads as a new view instead of a jump. Not on
    // each keystroke of the library search (too frequent). Opacity + Translate only.
    function relist() {
        listIn.stop()
        if (Theme.motion.fast <= 0) { playlists.opacity = 1; listShift.y = 0; return }
        listIn.start()
    }
    onCompactChanged: relist()
    onSortKeyChanged: relist()
    Connections {
        target: Backend.Library
        function onSidebarFilterChanged() { sidebar.relist() }
    }
    ParallelAnimation {
        id: listIn
        OpacityAnimator { target: playlists; from: 0; to: 1; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        NumberAnimation { target: listShift; property: "y"; from: Theme.space.xs; to: 0; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }

    // Small text button with a trailing chevron (the sort selector).
    component SortButton: T.AbstractButton {
        id: sortControl
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        implicitHeight: Theme.size.iconButton
        implicitWidth: sortRow.implicitWidth + 2 * Theme.space.sm
        Accessible.role: Accessible.Button
        Accessible.name: text
        readonly property bool hot: enabled && (hovered || pressed)
        scale: pressed ? Theme.feel.pressScale : 1
        Behavior on scale {
            ScaleAnimator { duration: sortControl.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        background: Rectangle {
            radius: Theme.radius.md
            color: Theme.surface2
            opacity: sortControl.hot ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, sortControl.hot ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            border.width: sortControl.visualFocus ? Theme.stroke.focus : 0
            border.color: Theme.focusRing
        }
        contentItem: Item {
            Row {
                id: sortRow
                anchors.centerIn: parent
                spacing: Theme.space.xs
                LibraryLabel {
                    anchors.verticalCenter: parent.verticalCenter
                    text: sortControl.text
                    color: sortControl.hot ? Theme.text : Theme.textSecondary
                    font.pixelSize: Theme.type.caption
                    font.weight: Theme.type.semibold
                }
                IconImage {
                    anchors.verticalCenter: parent.verticalCenter
                    source: Qt.resolvedUrl("../../icons/expand_more.svg")
                    sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                    color: sortControl.hot ? Theme.text : Theme.textSecondary
                }
            }
        }
    }

    // Art slot of a row: the cover (round for artists) or, without one, a soft accent plate with
    // the kind's icon (never an empty square). `liked` draws the «Tus me gusta» gradient tile.
    component EntryArt: Item {
        id: slot
        property string kind: "playlist"
        property string imageUrl: ""
        property bool liked: false
        readonly property bool round: kind === "artist"
        LibraryArtwork {
            anchors.fill: parent
            visible: slot.imageUrl !== "" && !slot.liked
            source: visible ? slot.imageUrl : ""
            portrait: slot.round
            fallback: sidebar.iconFor(slot.kind)
        }
        Rectangle {
            anchors.fill: parent
            visible: slot.imageUrl === "" || slot.liked
            radius: slot.round ? width / 2 : Theme.radius.sm
            color: Theme.accentSoft
            gradient: slot.liked ? likedGradient : null
            Gradient {
                id: likedGradient
                orientation: Gradient.Vertical
                GradientStop { position: 0; color: Theme.home.likedFrom }
                GradientStop { position: 1; color: Theme.home.likedTo }
            }
            IconImage {
                anchors.centerIn: parent
                source: slot.liked ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : sidebar.iconFor(slot.kind)
                sourceSize: Qt.size(Theme.icon.lg, Theme.icon.lg)
                color: slot.liked ? Theme.home.likedInk : Theme.accent
            }
        }
    }

    Column {
        id: fixedNav
        width: parent.width
        spacing: Theme.space.xs
        LibraryButton { width: parent.width; text: qsTr("Inicio"); iconSource: Qt.resolvedUrl("../../icons/home.svg"); selected: sidebar.currentPage === "home"; onClicked: sidebar.navigate("home", "", "") }
        // T3: focuses the search box of the top bar (Library.focusSearch) instead of an empty page.
        LibraryButton { objectName: "sidebarSearchButton"; width: parent.width; text: qsTr("Buscar"); iconSource: Qt.resolvedUrl("../../icons/search.svg"); selected: sidebar.currentPage === "search"; onClicked: sidebar.searchRequested() }
        Item { width: parent.width; height: Theme.space.sm }
        // «Biblioteca» header: search in the library, create playlist, compact view.
        Item {
            width: parent.width
            height: Theme.size.iconButton
            Row {
                x: Theme.space.sm
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.space.sm
                IconImage {
                    anchors.verticalCenter: parent.verticalCenter
                    source: Qt.resolvedUrl("../../icons/library_music.svg")
                    sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                    color: Theme.textSecondary
                }
                LibraryLabel {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.max(0, sidebar.width - 3 * Theme.size.iconButton - Theme.icon.md - 2 * Theme.space.sm)
                    text: qsTr("Biblioteca")
                    font.weight: Theme.type.semibold
                }
            }
            Row {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                IconButton {
                    objectName: "sidebarSearchButton"
                    text: qsTr("Buscar en tu biblioteca")
                    iconSource: Qt.resolvedUrl("../../icons/search.svg")
                    iconSize: Theme.icon.sm
                    ink: Theme.textSecondary
                    active: searchField.visible
                    activeInk: Theme.accent
                    onClicked: {
                        if (searchField.visible) sidebar.closeSearch()
                        else { sidebar.searchOpen = true; searchField.forceActiveFocus() }
                    }
                }
                IconButton {
                    objectName: "sidebarCreateButton"
                    text: qsTr("Crear playlist")
                    iconSource: Qt.resolvedUrl("../../icons/add.svg")
                    iconSize: Theme.icon.sm
                    ink: Theme.textSecondary
                    onClicked: { sidebar.creating = !sidebar.creating; if (sidebar.creating) playlistName.forceActiveFocus() }
                }
                IconButton {
                    objectName: "sidebarCompactButton"
                    text: sidebar.compact ? qsTr("Mostrar carátulas en la biblioteca") : qsTr("Vista compacta de la biblioteca")
                    iconSource: Qt.resolvedUrl("../../icons/queue_music.svg")
                    iconSize: Theme.icon.sm
                    ink: Theme.textSecondary
                    active: sidebar.compact
                    activeInk: Theme.accent
                    enabled: !sidebar.narrow   // G4: forced compact while dragged narrow
                    Accessible.checkable: true
                    onClicked: sidebar.toggleCompact()
                }
            }
        }
        // Library search (local filter, Library.searchLibrary), inline under the header.
        LibraryField {
            id: searchField
            objectName: "sidebarSearchField"
            width: parent.width
            visible: sidebar.searchOpen || Backend.Library.sidebarQuery !== ""
            leftPadding: Theme.space.md + Theme.icon.sm + Theme.space.sm
            placeholderText: qsTr("Buscar en tu biblioteca")
            onTextEdited: Backend.Library.searchLibrary(text)
            Keys.onEscapePressed: sidebar.closeSearch()
            onActiveFocusChanged: if (!activeFocus && text === "") sidebar.searchOpen = false
            // S1: fades in when opened (fast, easeOut); closing is instant.
            onVisibleChanged: if (visible && Theme.motion.fast > 0) searchIn.restart()
            OpacityAnimator { id: searchIn; target: searchField; from: 0; to: 1; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            IconImage {
                x: Theme.space.md
                anchors.verticalCenter: parent.verticalCenter
                source: Qt.resolvedUrl("../../icons/search.svg")
                sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                color: Theme.textSecondary
            }
        }
        Row {
            width: parent.width
            visible: sidebar.creating
            spacing: Theme.space.sm
            LibraryField {
                id: playlistName
                objectName: "newPlaylistName"
                width: parent.width - Theme.size.iconButton - Theme.space.sm
                placeholderText: qsTr("Nombre de la playlist")
                onAccepted: sidebar.create()
                Keys.onEscapePressed: sidebar.creating = false
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Crear playlist")
                iconSource: Qt.resolvedUrl("../../icons/check.svg")
                ink: Theme.accent
                enabled: playlistName.text.trim() !== ""
                onClicked: sidebar.create()
            }
        }
        // Kind chips; touching the active one clears the filter (as does the ×).
        Flickable {
            id: chipStrip
            objectName: "sidebarChips"
            width: parent.width
            height: Theme.library.chipHeight
            contentWidth: chipRow.width
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            Row {
                id: chipRow
                spacing: Theme.space.xs
                IconButton {
                    objectName: "sidebarClearFilter"
                    visible: Backend.Library.sidebarFilter !== "all"
                    width: Theme.library.chipHeight
                    height: width
                    text: qsTr("Quitar el filtro")
                    iconSource: Qt.resolvedUrl("../../icons/close.svg")
                    iconSize: Theme.icon.sm
                    restFill: Theme.surface2
                    hoverFill: Theme.surface3
                    onClicked: Backend.Library.setSidebarFilter("all")
                }
                Repeater {
                    model: [
                        { kind: "playlist", label: qsTr("Playlists") },
                        { kind: "album", label: qsTr("Álbumes") },
                        { kind: "artist", label: qsTr("Artistas") },
                        { kind: "show", label: qsTr("Pódcasts") }
                    ]
                    delegate: LibraryChip {
                        required property var modelData
                        objectName: "sidebarChip_" + modelData.kind
                        // Podcasts only when the library could read them (showsStatus).
                        visible: modelData.kind !== "show" || Backend.Library.showsStatus !== "unavailable"
                        leftPadding: Theme.space.md
                        rightPadding: Theme.space.md
                        text: modelData.label
                        selected: Backend.Library.sidebarFilter === modelData.kind
                        Accessible.name: qsTr("Mostrar solo %1").arg(modelData.label)
                        onClicked: Backend.Library.setSidebarFilter(selected ? "all" : modelData.kind)
                    }
                }
            }
        }
        Item {
            width: parent.width
            height: Theme.size.iconButton
            SortButton {
                id: sortButton
                objectName: "sidebarSortButton"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(implicitWidth, parent.width)
                text: sidebar.sortLabel(sidebar.sortKey)
                Accessible.name: qsTr("Ordenar por: %1").arg(text)
                onClicked: sortMenu.popup(sortButton, sortButton.width - sortMenu.width, sortButton.height)
            }
        }
    }
    // R3 (P2-8): the same choices as the list's «Clasificar por» (SortMenu.Choice): the label at
    // the left for every option, the check at the end of the active one.
    Menu {
        id: sortMenu
        objectName: "sidebarSortMenu"
        transformOrigin: Popup.Top
        enter: null
        exit: null
        property PopupEntrance entrance: PopupEntrance { popup: sortMenu; fromScale: Theme.entrance.menuScale; duration: Theme.motion.instant }
        implicitWidth: Theme.trackList.menuWidth
        padding: Theme.space.xs
        font.family: Theme.family
        font.pixelSize: Theme.type.body
        background: SortMenu.PopoverBackground {}
        SortMenu.SectionLabel { text: qsTr("Ordenar por") }
        SortMenu.Choice { objectName: "sidebarSortRecentPlayed"; text: sidebar.sortLabel("recent_played"); selected: sidebar.sortKey === "recent_played"; onTriggered: sidebar.setSort("recent_played") }
        SortMenu.Choice { objectName: "sidebarSortRecentAdded"; text: sidebar.sortLabel("recent_added"); selected: sidebar.sortKey === "recent_added"; onTriggered: sidebar.setSort("recent_added") }
        SortMenu.Choice { objectName: "sidebarSortAlphabetical"; text: sidebar.sortLabel("alphabetical"); selected: sidebar.sortKey === "alphabetical"; onTriggered: sidebar.setSort("alphabetical") }
        SortMenu.Choice { objectName: "sidebarSortCreator"; text: sidebar.sortLabel("creator"); selected: sidebar.sortKey === "creator"; onTriggered: sidebar.setSort("creator") }
    }
    // Lookup of a playlist's description for «Editar detalles» (sidebarItems has no such role).
    DelegateModel { id: playlistLookup; model: Backend.Library.playlists; delegate: Item {} }
    function playlistDescription(id) {
        for (let i = 0; i < playlistLookup.items.count; ++i) {
            const row = playlistLookup.items.get(i).model
            if (row.playlistId === id) return row.description
        }
        return ""
    }
    // DelegateModel (not the model directly) so previews can read the order without get().
    DelegateModel {
        id: entries
        model: Backend.Library.sidebarItems
        delegate: LibraryButton {
            id: entry
            required property int index
            objectName: "sidebarEntry" + index
            required property string kind
            required property string objectId
            required property string uri
            required property string name
            required property string creator
            required property string imageUrl
            required property bool pinned
            required property bool owned
            readonly property bool menuShown: playlistMenu.visible && playlistMenu.uri === uri
            // "…" while pointed at, focused or showing its menu; otherwise the pin, if pinned.
            readonly property bool showMore: hovered || more.hovered || activeFocus || more.activeFocus || menuShown
            width: playlists.width - Theme.space.sm
            height: sidebar.rowHeight
            text: name
            Accessible.name: qsTr("%1, %2").arg(name).arg(sidebar.subtitleFor(kind, creator))
            leftPadding: sidebar.compact ? Theme.space.md : Theme.space.sm
            rightPadding: Theme.size.iconButton
            selected: sidebar.currentPage === kind && sidebar.currentId === objectId
            onClicked: sidebar.open(kind, objectId, creator)
            // R3 (P1-2): the focused row (or its "…") scrolls into view, so Tab and ↑/↓ reach rows
            // the ListView has not created yet (the list only builds what is near the view).
            function reveal() {
                playlists.currentIndex = index
                playlists.positionViewAtIndex(index, ListView.Contain)
            }
            onActiveFocusChanged: if (activeFocus) reveal()
            function focusRow(target) {
                if (target < 0 || target >= entries.items.count) return
                playlists.currentIndex = target
                playlists.positionViewAtIndex(target, ListView.Contain)
                playlists.forceLayout()
                const row = playlists.itemAtIndex(target)
                if (row) row.forceActiveFocus(Qt.TabFocusReason)
            }
            Keys.onTabPressed: event => {
                if (index + 1 < entries.items.count) focusRow(index + 1)
                else event.accepted = false
            }
            Keys.onBacktabPressed: event => {
                if (index > 0) focusRow(index - 1)
                else if (playlists.headerItem && playlists.headerItem.visible)
                    playlists.headerItem.forceActiveFocus(Qt.BacktabFocusReason)
                else event.accepted = false
            }
            Keys.onUpPressed: event => {
                if (index > 0) focusRow(index - 1)
                else if (playlists.headerItem && playlists.headerItem.visible) playlists.headerItem.forceActiveFocus(Qt.TabFocusReason)
            }
            Keys.onDownPressed: focusRow(index + 1)
            contentItem: Row {
                spacing: Theme.space.md
                EntryArt {
                    visible: !sidebar.compact
                    anchors.verticalCenter: parent.verticalCenter
                    width: sidebar.artSize
                    height: width
                    kind: entry.kind
                    imageUrl: sidebar.compact ? "" : entry.imageUrl
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    width: entry.availableWidth - (sidebar.compact ? 0 : sidebar.artSize + parent.spacing)
                    spacing: Theme.space.xxs
                    LibraryLabel {
                        width: parent.width
                        text: entry.name
                        color: entry.selected ? Theme.accent : Theme.text
                        font.weight: Theme.type.medium
                    }
                    Row {
                        visible: !sidebar.compact
                        width: parent.width
                        spacing: Theme.space.xs
                        IconImage {
                            visible: entry.pinned
                            anchors.verticalCenter: parent.verticalCenter
                            source: Qt.resolvedUrl("../../icons/push_pin_fill.svg")
                            sourceSize: Qt.size(Theme.type.caption, Theme.type.caption)
                            color: Theme.accent
                            Accessible.role: Accessible.StaticText
                            Accessible.name: qsTr("Fijada")
                        }
                        LibraryLabel {
                            width: parent.width - (entry.pinned ? Theme.type.caption + parent.spacing : 0)
                            text: sidebar.subtitleFor(entry.kind, entry.creator)
                            color: Theme.textSecondary
                            font.pixelSize: Theme.type.caption
                        }
                    }
                }
            }
            function openMenu(atPointer) {
                playlistMenu.kind = kind
                playlistMenu.objectId = objectId
                playlistMenu.uri = uri
                playlistMenu.name = name
                playlistMenu.owned = owned
                playlistMenu.pinned = pinned
                playlistMenu.description = kind === "playlist" ? sidebar.playlistDescription(objectId) : ""
                if (atPointer) playlistMenu.popup()
                // Under the "…", opening toward the page (the sidebar is narrower than the menu).
                else playlistMenu.popup(entry, entry.width - Theme.size.iconButton, entry.height)
            }
            // M1b: right click, the Menu key, Shift+F10 and the "…" button open the same menu.
            Keys.onMenuPressed: openMenu(false)
            Keys.onPressed: event => {
                if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
                    openMenu(false); event.accepted = true
                }
            }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: entry.openMenu(true)
            }
            // Compact rows have no subtitle line: the pin sits at the right instead.
            IconImage {
                anchors.right: parent.right
                anchors.rightMargin: (Theme.size.iconButton - Theme.icon.sm) / 2
                anchors.verticalCenter: parent.verticalCenter
                visible: entry.pinned && sidebar.compact
                // Crossfades with the "…" below (same timing, inverted).
                opacity: entry.showMore ? 0 : 1
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, entry.showMore ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                source: Qt.resolvedUrl("../../icons/push_pin_fill.svg")
                sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                color: Theme.accent
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Fijada")
            }
            IconButton {
                id: more
                objectName: "sidebarPlaylistMore"
                // The row is the only Tab stop; Menu / Shift+F10 expose the same actions.
                focusPolicy: Qt.NoFocus
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                // Fades with the row's hover (opacity only): in at `instant`, out at `fast`.
                opacity: entry.showMore ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, entry.showMore ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                text: qsTr("Más opciones para %1").arg(entry.name)
                iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                iconSize: Theme.icon.sm
                ink: Theme.textSecondary
                onClicked: entry.openMenu(false)
                onActiveFocusChanged: if (activeFocus) entry.reveal()
            }
        }
    }
    // M1b: one menu for every row (delegates stay light); the row fills it in on open.
    LibraryMenu {
        id: playlistMenu
        objectName: "sidebarPlaylistMenu"
        property string kind: "playlist"
        property string objectId: ""
        property string uri: ""
        property string name: ""
        property string description: ""
        property bool owned: false
        property bool pinned: false
        readonly property string playlistId: kind === "playlist" ? objectId : ""
        seedUri: kind === "show" ? "" : uri
        Action {
            text: playlistMenu.kind === "album" ? (playlistMenu.pinned ? qsTr("Desfijar álbum") : qsTr("Fijar álbum"))
                : playlistMenu.kind === "artist" ? (playlistMenu.pinned ? qsTr("Desfijar artista") : qsTr("Fijar artista"))
                : playlistMenu.kind === "show" ? (playlistMenu.pinned ? qsTr("Desfijar pódcast") : qsTr("Fijar pódcast"))
                : (playlistMenu.pinned ? qsTr("Desfijar playlist") : qsTr("Fijar playlist"))
            enabled: playlistMenu.uri !== ""
            onTriggered: Backend.Library.toggleLibraryPin(playlistMenu.uri)
        }
        Action {
            text: qsTr("Editar detalles")
            enabled: playlistMenu.kind === "playlist" && playlistMenu.owned
            onTriggered: editDialog.openFor(playlistMenu.objectId, playlistMenu.name, playlistMenu.description)
        }
        Action {
            text: qsTr("Eliminar")
            enabled: playlistMenu.kind === "playlist" && playlistMenu.owned
            onTriggered: sidebar.confirmDelete(playlistMenu.objectId, playlistMenu.name)
        }
        Action {
            text: qsTr("Quitar de tu biblioteca")
            enabled: !playlistMenu.owned && playlistMenu.kind !== "artist" && playlistMenu.uri !== ""
            onTriggered: Backend.Actions.setSaved(playlistMenu.uri, false)
        }
        Action {
            text: qsTr("Dejar de seguir")
            enabled: playlistMenu.kind === "artist" && playlistMenu.objectId !== ""
            onTriggered: Backend.Library.setArtistFollowed(playlistMenu.objectId, false)
        }
    }
    function confirmDelete(id, name) {
        deleteDialog.playlistId = id
        deleteDialog.message = qsTr("Esto eliminará «%1» de tu biblioteca.").arg(name)
        deleteDialog.open()
    }
    readonly property alias playlistMenu: playlistMenu
    readonly property alias sortMenu: sortMenu
    readonly property alias editDialog: editDialog
    readonly property alias deleteDialog: deleteDialog
    PlaylistEditDialog { id: editDialog }
    LibraryConfirmDialog {
        id: deleteDialog
        objectName: "playlistDeleteDialog"
        property string playlistId: ""
        title: qsTr("¿Eliminar de tu biblioteca?")
        confirmText: qsTr("Eliminar")
        // In Spotify deleting your playlist is unfollowing it; Library.qml leaves the page if it was open.
        onConfirmed: Backend.Actions.setSaved("spotify:playlist:" + playlistId, false)
    }
    readonly property string sidebarStatus: Backend.Library.sidebarStatus
    readonly property bool likedShown: {
        const filter = Backend.Library.sidebarFilter
        const query = String(Backend.Library.sidebarQuery).trim().toLowerCase()
        return (filter === "all" || filter === "playlist") && (query === "" || qsTr("Tus me gusta").toLowerCase().indexOf(query) >= 0)
    }
    ListView {
        id: playlists
        objectName: "sidebarPlaylists"
        anchors.top: fixedNav.bottom
        anchors.topMargin: Theme.space.xs
        anchors.bottom: account.top
        anchors.bottomMargin: Theme.space.md
        width: parent.width
        clip: true
        reuseItems: true
        model: entries
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        transform: Translate { id: listShift }
        // «Tus me gusta» heads the list (it is not part of sidebarItems), pinned as in Spotify.
        header: LibraryButton {
            id: liked
            objectName: "sidebarLiked"
            width: playlists.width - Theme.space.sm
            height: sidebar.likedShown ? sidebar.rowHeight : 0
            visible: sidebar.likedShown
            text: qsTr("Tus me gusta")
            leftPadding: sidebar.compact ? Theme.space.md : Theme.space.sm
            selected: sidebar.currentPage === "liked"
            onClicked: sidebar.navigate("liked", "", "")
            onActiveFocusChanged: if (activeFocus) playlists.positionViewAtBeginning()
            Keys.onTabPressed: event => {
                if (playlists.count > 0) {
                    playlists.currentIndex = 0
                    playlists.positionViewAtIndex(0, ListView.Contain)
                    playlists.forceLayout()
                    if (playlists.currentItem) playlists.currentItem.forceActiveFocus(Qt.TabFocusReason)
                } else event.accepted = false
            }
            Keys.onDownPressed: {
                playlists.currentIndex = 0
                playlists.positionViewAtIndex(0, ListView.Contain)
                playlists.forceLayout()
                const row = playlists.itemAtIndex(0)
                if (row) row.forceActiveFocus(Qt.TabFocusReason)
            }
            contentItem: Row {
                spacing: Theme.space.md
                EntryArt {
                    visible: !sidebar.compact
                    anchors.verticalCenter: parent.verticalCenter
                    width: sidebar.artSize
                    height: width
                    liked: true
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    width: liked.availableWidth - (sidebar.compact ? 0 : sidebar.artSize + parent.spacing)
                    spacing: Theme.space.xxs
                    LibraryLabel { width: parent.width; text: liked.text; color: liked.selected ? Theme.accent : Theme.text; font.weight: Theme.type.medium }
                    Row {
                        visible: !sidebar.compact
                        spacing: Theme.space.xs
                        IconImage {
                            anchors.verticalCenter: parent.verticalCenter
                            source: Qt.resolvedUrl("../../icons/push_pin_fill.svg")
                            sourceSize: Qt.size(Theme.type.caption, Theme.type.caption)
                            color: Theme.accent
                        }
                        LibraryLabel { text: qsTr("Playlist"); color: Theme.textSecondary; font.pixelSize: Theme.type.caption }
                    }
                }
            }
        }
        footer: Column {
            width: playlists.width - Theme.space.sm
            topPadding: Theme.space.sm
            spacing: Theme.space.sm
            // Loading: a quiet skeleton (static plates, no shimmer: nothing runs while it waits).
            Repeater {
                model: sidebar.sidebarStatus === "loading" && entries.items.count === 0 ? Theme.library.skeletonRows : 0
                delegate: Row {
                    x: Theme.space.sm
                    spacing: Theme.space.md
                    opacity: Theme.library.disabledOpacity
                    Rectangle { visible: !sidebar.compact; width: sidebar.artSize; height: width; radius: Theme.radius.sm; color: Theme.surface2 }
                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.space.sm
                        Rectangle { width: Theme.size.coverMin / 2; height: Theme.space.md; radius: height / 2; color: Theme.surface2 }
                        Rectangle { visible: !sidebar.compact; width: Theme.size.coverMin / 3; height: Theme.space.sm; radius: height / 2; color: Theme.surface2 }
                    }
                }
            }
            LibraryLabel {
                objectName: "sidebarEmpty"
                x: Theme.space.sm
                width: parent.width - Theme.space.lg
                visible: entries.items.count === 0 && sidebar.sidebarStatus !== "loading" && sidebar.sidebarStatus !== "unavailable"
                         && (Backend.Library.sidebarQuery !== "" || Backend.Library.sidebarFilter !== "all")
                text: Backend.Library.sidebarQuery !== ""
                      ? qsTr("Nada en tu biblioteca coincide con «%1».").arg(Backend.Library.sidebarQuery)
                      : qsTr("Todavía no hay nada aquí.")
                color: Theme.textSecondary
                font.pixelSize: Theme.type.caption
                wrapMode: Text.Wrap
            }
            LibraryButton {
                visible: entries.items.count === 0 && sidebar.sidebarStatus === "unavailable"
                width: parent.width
                text: qsTr("Volver a cargar tu biblioteca")
                onClicked: Backend.Library.refreshSidebar()
            }
        }
    }
    LibraryButton {
        id: account
        objectName: "accountMenuButton"
        anchors.bottom: parent.bottom
        width: parent.width
        text: Backend.Account.displayName !== "" ? Backend.Account.displayName : qsTr("Mi cuenta")
        Accessible.name: qsTr("Cuenta de %1").arg(text)
        contentItem: Row {
            spacing: Theme.space.sm
            LibraryArtwork {
                objectName: "sidebarAvatar"
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.size.iconButton - Theme.space.sm
                height: width
                portrait: true
                source: Backend.Account.avatarUrl
                fallback: Qt.resolvedUrl("../../icons/person.svg")
            }
            LibraryLabel {
                anchors.verticalCenter: parent.verticalCenter
                width: account.availableWidth - Theme.size.iconButton
                text: account.text; font.weight: Theme.type.semibold
            }
        }
        onClicked: accountMenu.open()   // anchored above the button (x/y), not at the pointer
    }
    readonly property alias accountMenu: accountMenu
    LibraryMenu {
        id: accountMenu; objectName: "accountMenu"; seedActions: false
        x: 0; y: account.y - height - Theme.space.sm
        transformOrigin: Popup.BottomLeft
        Action { text: qsTr("Ver perfil"); icon.source: Qt.resolvedUrl("../../icons/account_circle.svg"); onTriggered: sidebar.navigate("profile", "me", "") }
        Action { text: qsTr("Cerrar sesión"); icon.source: Qt.resolvedUrl("../../icons/logout.svg"); onTriggered: signOutDialog.open() }
    }
    LibraryConfirmDialog {
        id: signOutDialog; objectName: "signOutConfirm"
        title: qsTr("¿Cerrar sesión?")
        message: qsTr("Se detendrá la música y se borrarán las autorizaciones de Spotify de este PC.")
        confirmText: qsTr("Cerrar sesión")
        onConfirmed: Backend.Account.signOut()
    }
}
