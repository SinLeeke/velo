import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"
import ".."

// F6: artist profile like the official app. Header (portrait, name, Reproducir / Aleatorio /
// Seguir), "Populares" (5, "Ver más" up to 10), "Discografía" with type chips, "Fans también
// escuchan", "Aparece en" and "Acerca de" (photos + biography). The extra sections come from the
// librespot session (Library.artist*); while that profile is not ready the page keeps the Web API
// header and discography (Library.albums) and hides the rest. Spotify does not give monthly
// listeners, cities or the verified badge through this route, so none of that is shown.
Item {
    id: page
    property string artistId: ""
    signal albumRequested(string albumId)
    signal artistRequested(string artistId)
    signal addToPlaylistRequested(string uri)
    TrackMenu { id: songMenu }
    readonly property bool profileReady: Backend.Library.artistStatus === "ready"
    readonly property string artistUri: artistId !== "" ? "spotify:artist:" + artistId : ""
    property bool popularExpanded: false
    property bool bioExpanded: false
    property string localLoadError: ""
    readonly property string loadError: "loadError" in Backend.Library && Backend.Library.loadError !== ""
        ? Backend.Library.loadError : localLoadError
    function reload() { localLoadError = ""; Backend.Library.openArtist(artistId) }
    Connections {
        target: Backend.Player
        function onError(message) {
            if (page.visible && (Backend.Library.loading || Backend.Library.artistStatus === "loading")) page.localLoadError = message
        }
    }
    Connections {
        target: Backend.Library
        function onLoadingChanged() { if (Backend.Library.loading) page.localLoadError = "" }
    }
    // -1 unknown (asking), 0 not following, 1 following. Kept here so a menu that inspects
    // another URI through the shared Actions singleton does not flip this button.
    property int following: -1
    property bool initialized: false
    function refreshFollowing() {
        following = -1
        if (artistUri !== "") Backend.Actions.inspectSaved(artistUri)
    }
    onArtistIdChanged: { popularExpanded = false; bioExpanded = false; localLoadError = ""; if (initialized) refreshFollowing() }
    Component.onCompleted: { initialized = true; refreshFollowing() }
    Connections {
        target: Backend.Actions
        function onInspectedSavedChanged() {
            if (Backend.Actions.inspectedUri === page.artistUri && Backend.Actions.inspectedSaved >= 0)
                page.following = Backend.Actions.inspectedSaved
        }
        function onSavedChanged(uri, saved) { if (uri === page.artistUri) page.following = saved ? 1 : 0 }
    }

    // "Populares" columns, derived once per page width (same rules as TrackListPage).
    TextMetrics { id: numberMetrics; text: "10"; font.family: Theme.family; font.pixelSize: Theme.type.body; font.features: Theme.tabularFeatures }
    readonly property real numberWidth: Math.ceil(numberMetrics.advanceWidth)
    readonly property real rowWidth: content.width - Theme.space.sm
    readonly property real artWidth: Theme.trackList.rowArt
    readonly property real flexWidth: rowWidth - 2 * Theme.trackList.rowInset - numberWidth - artWidth - Theme.space.md
        - Theme.trackList.trailingWidth - 4 * Theme.space.md
    readonly property real albumWidth: rowWidth >= Theme.trackList.albumMinRowWidth ? Math.round(flexWidth * Theme.trackList.albumShare) : 0
    readonly property real titleWidth: Math.max(0, flexWidth - (albumWidth > 0 ? albumWidth + Theme.space.md : 0))

    component SectionTitle: LibraryLabel {
        font.pixelSize: Theme.type.heading
        font.weight: Theme.type.semibold
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    // Horizontal shelf of square cards (albums, singles, "Aparece en").
    component AlbumShelf: ListView {
        id: shelf
        property bool showType: false
        signal opened(string albumId)
        x: -Theme.space.sm
        width: content.width + 2 * Theme.library.cardBleed
        height: Theme.library.cardWidth + Theme.size.trackRow + Theme.space.md
        orientation: ListView.Horizontal
        spacing: Theme.space.md
        reuseItems: true
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.horizontal: LibraryScrollBar { height: Theme.space.sm; policy: shelf.contentWidth > shelf.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff }
        WheelHandler {
            onWheel: event => { if (event.angleDelta.y !== 0) { viewport.scrollWheel(event.angleDelta.y); event.accepted = true } }
        }
        delegate: MediaCard {
            required property int index
            required property string albumId
            required property string name
            required property string imageUrl
            required property int year
            required property var model
            readonly property string typeLabel: {
                switch (model.albumType) {
                case "album": return qsTr("Álbum")
                case "single": return qsTr("Sencillo")
                case "ep": return qsTr("EP")
                case "compilation": return qsTr("Recopilación")
                default: return ""
                }
            }
            width: Theme.library.cardWidth
            artSource: imageUrl
            title: name
            subtitle: shelf.showType && typeLabel !== ""
                      ? (year > 0 ? qsTr("%1 · %2").arg(year).arg(typeLabel) : typeLabel)
                      : (year > 0 ? qsTr("%1 · %2").arg(year).arg(model.artist) : model.artist)
            uri: "spotify:album:" + albumId
            onActiveFocusChanged: { if (activeFocus) { shelf.currentIndex = index; viewport.revealCard(shelf, index) } }
            Keys.onPressed: event => viewport.tabCard(shelf, event, index)
            onClicked: shelf.opened(albumId)
        }
    }

    Flickable {
        id: viewport
        x: -Theme.library.cardBleed
        width: page.width + 2 * Theme.library.cardBleed
        height: page.height
        contentWidth: width
        contentHeight: content.height + Theme.space.xl
        flickableDirection: Flickable.VerticalFlick
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        ScrollBar.vertical: LibraryScrollBar {}
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
        function revealItem(item) {
            const top = item.mapToItem(contentItem, 0, 0).y
            const bottom = top + item.height
            contentY = Math.max(originY, Math.min(top < contentY ? top : bottom > contentY + height ? bottom - height : contentY,
                originY + Math.max(0, contentHeight - height)))
        }
        function revealCard(list, index) {
            list.positionViewAtIndex(index, ListView.Contain)
            const top = list.mapToItem(viewport.contentItem, 0, 0).y
            const bottom = top + list.height
            const target = top < contentY ? top : bottom > contentY + height ? bottom - height : contentY
            // S1: keyboard focus reveals cards immediately, without scroll animation.
            contentY = Math.max(originY, Math.min(target, originY + Math.max(0, contentHeight - height)))
        }
        Connections {
            target: Backend.Library
            function onCurrentTitleChanged() { viewport.contentY = viewport.originY }
        }

        Column {
            id: content
            x: Theme.library.cardBleed
            width: page.width - Theme.space.md
            spacing: Theme.space.xxl

            // ---- header
            Column {
                width: parent.width
                spacing: Theme.space.xl
                LibraryPageHeader {
                    width: parent.width
                    kindLabel: qsTr("Artista")
                    title: Backend.Library.currentTitle
                    titleObjectName: "artistTitle"
                    artSource: Backend.Library.currentImageUrl
                    portrait: true
                    fallback: Qt.resolvedUrl("../../icons/person.svg")
                    meta: [ LibraryLabel { width: parent.width; visible: text !== ""; text: Backend.Library.currentSubtitle; color: Theme.textSecondary; wrapMode: Text.Wrap } ]
                }
                Flow {
                    width: parent.width
                    spacing: Theme.space.lg
                    LibraryButton {
                        objectName: "artistPlayButton"
                        width: Theme.library.cardWidth
                        primary: true
                        text: qsTr("Reproducir")
                        iconSource: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                        enabled: Backend.Player.canControl
                        onClicked: Backend.Player.shuffle ? Backend.Library.shuffleCurrent()
                            : popularList.count > 0 ? Backend.Library.playArtistPopular(0) : Backend.Library.playCurrent()
                    }
                    IconButton {
                        objectName: "artistShuffleSwitch"
                        width: Theme.size.titleBar
                        height: Theme.size.titleBar
                        text: Backend.Player.shuffle ? qsTr("Desactivar aleatorio") : qsTr("Activar aleatorio")
                        iconSource: Qt.resolvedUrl("../../icons/shuffle.svg")
                        iconSize: Theme.icon.lg
                        active: Backend.Player.shuffle
                        activeFill: Theme.library.clearPaint
                        activeInk: Theme.accent
                        Accessible.checkable: true
                        enabled: Backend.Player.canControl
                        onClicked: Backend.Player.toggleShuffle()
                    }
                    LibraryButton {
                        objectName: "artistFollowButton"
                        text: page.following === 1 ? qsTr("Siguiendo") : qsTr("Seguir")
                        selected: page.following === 1
                        Accessible.checkable: true
                        Accessible.checked: page.following === 1
                        iconSource: page.following === 1 ? Qt.resolvedUrl("../../icons/check.svg") : Qt.resolvedUrl("../../icons/person.svg")
                        // Unknown state (still asking Spotify): disabled rather than guessing.
                        enabled: page.following >= 0 && page.artistUri !== ""
                        onClicked: Backend.Actions.setSaved(page.artistUri, page.following !== 1)
                    }
                    IconButton {
                        id: artistMore
                        width: Theme.size.titleBar
                        height: Theme.size.titleBar
                        text: qsTr("Más opciones de %1").arg(Backend.Library.currentTitle)
                        iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                        onClicked: { artistMenu.active = true; const menu = artistMenu.item as MediaMenu; menu.popup(artistMore, 0, artistMore.height) }
                    }
                    Loader { id: artistMenu; active: false; sourceComponent: MediaMenu { uri: page.artistUri } }
                }
                LibraryLabel {
                    visible: Backend.Library.artistStatus === "loading"
                    text: qsTr("Cargando el perfil del artista…")
                    color: Theme.textSecondary
                    font.pixelSize: Theme.type.caption
                }
                LibraryLabel { width: parent.width; visible: page.loadError !== "" && !Backend.Library.loading; text: page.loadError; color: Theme.textSecondary; wrapMode: Text.Wrap }
                LibraryButton { visible: page.loadError !== "" && !Backend.Library.loading; text: qsTr("Reintentar"); onClicked: page.reload() }
            }

            // ---- Populares
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: page.profileReady && popularList.count > 0
                SectionTitle { text: qsTr("Populares") }
                ListView {
                    id: popularList
                    objectName: "artistPopularList"
                    width: parent.width
                    readonly property int shown: page.popularExpanded ? count : Math.min(count, Theme.artistPage.popularCollapsed)
                    height: shown * Theme.size.trackRow
                    interactive: false
                    clip: true
                    reuseItems: true
                    model: Backend.Library.artistPopular
                    delegate: TrackRow {
                        visible: index < popularList.shown
                        width: popularList.width - Theme.space.sm
                        numberWidth: page.numberWidth
                        titleWidth: page.titleWidth
                        albumWidth: page.albumWidth
                        addedWidth: 0
                        keyboardCount: popularList.shown
                        sharedMenu: songMenu
                        onActiveFocusChanged: if (activeFocus) { popularList.currentIndex = index; viewport.revealItem(this) }
                        playAction: i => Backend.Library.playArtistPopular(i)
                        onSelected: rowIndex => popularList.currentIndex = rowIndex
                        onAddToPlaylistRequested: uri => page.addToPlaylistRequested(uri)
                    }
                }
                LibraryButton {
                    visible: popularList.count > Theme.artistPage.popularCollapsed
                    text: page.popularExpanded ? qsTr("Ver menos") : qsTr("Ver más canciones")
                    onClicked: page.popularExpanded = !page.popularExpanded
                }
            }

            // ---- Discografía (librespot profile by type; Web API albums until it is ready)
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: discography.count > 0 || (page.profileReady && Backend.Library.artistDiscographyType !== "all")
                SectionTitle { text: qsTr("Discografía") }
                Row {
                    visible: page.profileReady
                    spacing: Theme.space.sm
                    Repeater {
                        model: [
                            { kind: "all", label: qsTr("Todo") },
                            { kind: "album", label: qsTr("Álbumes") },
                            { kind: "single", label: qsTr("Sencillos y EP") },
                            { kind: "compilation", label: qsTr("Recopilaciones") }
                        ]
                        delegate: LibraryChip {
                            required property var modelData
                            text: modelData.label
                            selected: Backend.Library.artistDiscographyType === modelData.kind
                            onClicked: Backend.Library.setArtistDiscographyType(modelData.kind)
                        }
                    }
                }
                AlbumShelf {
                    id: discography
                    showType: page.profileReady
                    model: page.profileReady ? Backend.Library.artistDiscography : Backend.Library.albums
                    onOpened: albumId => page.albumRequested(albumId)
                }
                LibraryLabel {
                    visible: page.profileReady && discography.count === 0
                    text: qsTr("No hay lanzamientos de este tipo.")
                    color: Theme.textSecondary
                }
            }

            // ---- Fans también escuchan
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: page.profileReady && relatedList.count > 0
                SectionTitle { text: qsTr("Fans también escuchan") }
                ListView {
                    id: relatedList
                    x: -Theme.space.sm
                    width: parent.width + 2 * Theme.library.cardBleed
                    height: Theme.library.cardWidth + Theme.size.trackRow + Theme.space.md
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.horizontal: LibraryScrollBar { height: Theme.space.sm; policy: relatedList.contentWidth > relatedList.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff }
                    WheelHandler { onWheel: event => { if (event.angleDelta.y !== 0) { viewport.scrollWheel(event.angleDelta.y); event.accepted = true } } }
                    model: Backend.Library.artistRelated
                    delegate: MediaCard {
                        required property int index
                        required property string artistId
                        required property string name
                        required property string imageUrl
                        width: Theme.library.cardWidth
                        artSource: imageUrl
                        title: name
                        subtitle: qsTr("Artista")
                        portrait: true
                        uri: "spotify:artist:" + artistId
                        onActiveFocusChanged: { if (activeFocus) { relatedList.currentIndex = index; viewport.revealCard(relatedList, index) } }
                        Keys.onPressed: event => viewport.tabCard(relatedList, event, index)
                        onClicked: page.artistRequested(artistId)
                    }
                }
            }

            // ---- Aparece en
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: page.profileReady && appearsOn.count > 0
                SectionTitle { text: qsTr("Aparece en") }
                AlbumShelf {
                    id: appearsOn
                    model: Backend.Library.artistAppearsOn
                    onOpened: albumId => page.albumRequested(albumId)
                }
            }

            // ---- Acerca de
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: page.profileReady && (Backend.Library.artistBiography !== "" || photos.count > 0)
                SectionTitle { text: qsTr("Acerca de") }
                ListView {
                    id: photos
                    width: parent.width
                    height: count > 0 ? Theme.artistPage.aboutPhoto + Theme.space.md : 0
                    visible: count > 0
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.horizontal: LibraryScrollBar { height: Theme.space.sm; policy: photos.contentWidth > photos.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff }
                    WheelHandler { onWheel: event => { if (event.angleDelta.y !== 0) { viewport.scrollWheel(event.angleDelta.y); event.accepted = true } } }
                    model: Backend.Library.artistPhotos
                    delegate: LibraryArtwork {
                        required property string imageUrl
                        width: Theme.artistPage.aboutPhoto
                        height: width
                        source: imageUrl
                        fallback: Qt.resolvedUrl("../../icons/person.svg")
                    }
                }
                LibraryLabel {
                    id: bio
                    width: Math.min(parent.width, Theme.library.readableWidth)
                    visible: text !== ""
                    // Spotify's text may carry markup: shown as plain text, never interpreted.
                    textFormat: Text.PlainText
                    text: Backend.Library.artistBiography
                    wrapMode: Text.Wrap
                    elide: Text.ElideRight
                    // Expanded: no practical limit (Spotify biographies are a few paragraphs).
                    maximumLineCount: page.bioExpanded ? Theme.library.bioExpandedLines : Theme.artistPage.bioLines
                }
                LibraryButton {
                    visible: bio.visible && (bio.truncated || page.bioExpanded)
                    text: page.bioExpanded ? qsTr("Leer menos") : qsTr("Leer más")
                    onClicked: page.bioExpanded = !page.bioExpanded
                }
            }

        }
    }
}
