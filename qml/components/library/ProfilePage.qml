import QtQuick
import QtQuick.Controls.Basic
import Velo.Backend as Backend
import "../../theme"
import ".."

// U4: user profile like the official app ("me" = the signed-in account). Header: round avatar,
// "Perfil", the name, "N playlists públicas • N seguidores" (each only when Spotify says it).
// The own profile adds "Artistas más escuchados este mes" (Library.profileTopArtists,
// short_term) and "Canciones más escuchadas este mes" (Library.topTracks, the same list as
// Inicio). Everyone's: "Playlists públicas" (Library.profilePlaylists). No chats: Spotify has no
// messaging API. Sections enter when their data arrives (opacity + 12 px rise, `base`,
// ease-out, 40 ms apart; one shot, render thread except the Translate).
Item {
    id: page
    property string userId: "me"
    signal navigate(string kind, string itemId, string owner)
    signal addToPlaylistRequested(string uri)
    TrackMenu { id: songMenu }
    ShareMenu { id: profileMenu; uri: Backend.Library.profileUserId !== "" ? "spotify:user:" + Backend.Library.profileUserId : "" }
    readonly property string status: Backend.Library.profileStatus
    readonly property bool isMe: Backend.Library.profileIsMe
    readonly property bool ready: status === "ready"
    property string failure: ""
    onStatusChanged: if (status === "loading") failure = ""
    Connections {
        target: Backend.Player
        function onError(message) { if (page.visible && page.status === "loading") page.failure = message }
    }

    function summaryText() {
        const parts = []
        const lists = Backend.Library.profilePlaylistCount
        if (lists >= 0)
            parts.push(lists === 1 ? qsTr("1 playlist pública") : qsTr("%1 playlists públicas").arg(Number(lists).toLocaleString(Qt.locale(), "f", 0)))
        const followers = Backend.Library.profileFollowers
        if (followers >= 0)
            parts.push(followers === 1 ? qsTr("1 seguidor") : qsTr("%1 seguidores").arg(Number(followers).toLocaleString(Qt.locale(), "f", 0)))
        return parts.join(qsTr(" · "))
    }

    // "Canciones más escuchadas" columns, derived once per page width (same rules as the lists).
    TextMetrics { id: numberMetrics; text: "0"; font.family: Theme.family; font.pixelSize: Theme.type.body; font.features: Theme.tabularFeatures }
    readonly property real numberWidth: Math.ceil(numberMetrics.advanceWidth)
    readonly property real rowWidth: content.width
    readonly property real flexWidth: rowWidth - 2 * Theme.trackList.rowInset - numberWidth - Theme.trackList.rowArt - Theme.space.md
        - Theme.trackList.trailingWidth - 4 * Theme.space.md
    readonly property real albumWidth: rowWidth >= Theme.trackList.albumMinRowWidth ? Math.round(flexWidth * Theme.trackList.albumShare) : 0
    readonly property real titleWidth: Math.max(0, flexWidth - (albumWidth > 0 ? albumWidth + Theme.space.md : 0))

    component SectionTitle: LibraryLabel {
        font.pixelSize: Theme.type.heading
        font.weight: Theme.type.semibold
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    // "Solo tú puedes verlo" under the sections only the owner sees, as in Spotify.
    component PrivateNote: LibraryLabel {
        text: qsTr("Solo tú puedes verlo")
        color: Theme.textSecondary
        font.pixelSize: Theme.type.caption
    }
    // A section that rises in the first time it becomes visible (its data arrived). A reused or
    // already shown section stays put; with Theme.motion at 0 it is simply there.
    component Section: Column {
        id: section
        property int order: 0
        property bool shown: false
        width: parent ? parent.width : 0
        spacing: Theme.space.md
        transform: Translate { id: rise }
        onVisibleChanged: if (visible) reveal()
        Component.onCompleted: if (visible) reveal()
        function reveal() {
            if (shown) return
            shown = true
            if (Theme.motion.base <= 0) return
            hold.duration = Math.min(order, Theme.entrance.staggerMax) * Theme.entrance.stagger
            rise.y = Theme.entrance.cardShift
            entrance.restart()
        }
        SequentialAnimation {
            id: entrance
            OpacityAnimator { id: hold; target: section; from: 0; to: 0 }
            ParallelAnimation {
                OpacityAnimator { target: section; from: 0; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                NumberAnimation { target: rise; property: "y"; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
    }
    // Horizontal shelf of cards (artists round, playlists square).
    component Shelf: ListView {
        id: shelf
        x: -Theme.space.sm
        width: content.width + 2 * Theme.library.cardBleed
        height: Theme.library.cardWidth + Theme.size.trackRow + Theme.space.md
        orientation: ListView.Horizontal
        spacing: Theme.space.md
        reuseItems: true
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.horizontal: LibraryScrollBar { height: Theme.space.sm; policy: shelf.contentWidth > shelf.width ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff }
        WheelHandler { onWheel: event => { if (event.angleDelta.y !== 0) { viewport.scrollWheel(event.angleDelta.y); event.accepted = true } } }
    }

    Flickable {
        id: viewport
        objectName: "profileViewport"
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
            contentY = Math.max(originY, Math.min(target, originY + Math.max(0, contentHeight - height)))
        }

        Column {
            id: content
            x: Theme.library.cardBleed
            width: page.width - Theme.space.md
            spacing: Theme.space.xxl

            // ---- header
            LibraryPageHeader {
                width: parent.width
                kindLabel: qsTr("Perfil")
                title: Backend.Library.profileName
                titleObjectName: "profileName"
                artSource: Backend.Library.profileImageUrl
                artObjectName: "profileAvatar"
                portrait: true
                fallback: Qt.resolvedUrl("../../icons/person.svg")
                meta: [
                    LibraryLabel {
                        objectName: "profileSummary"
                        width: parent.width
                        visible: text !== ""
                        text: page.ready ? page.summaryText()
                              : page.status === "loading" ? qsTr("Cargando perfil…")
                              : ""
                        color: Theme.textSecondary
                        wrapMode: Text.Wrap
                    }
                ]
            }
            // «…» con Copiar enlace (Velo no sigue perfiles: Spotify no da esa API de seguir usuarios).
            IconButton {
                id: profileMore
                objectName: "profileMoreButton"
                visible: page.ready && Backend.Library.profileUserId !== ""
                width: Theme.size.titleBar
                height: Theme.size.titleBar
                text: qsTr("Más opciones de %1").arg(Backend.Library.profileName)
                iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                iconSize: Theme.icon.lg
                ink: Theme.textSecondary
                onClicked: profileMenu.popup(profileMore, 0, profileMore.height)
            }
            LibraryLabel { width: parent.width; visible: page.failure !== "" && page.status !== "loading"; text: page.failure; color: Theme.textSecondary; wrapMode: Text.Wrap }
            LibraryButton { visible: page.failure !== "" && page.status !== "loading"; text: qsTr("Reintentar"); onClicked: Backend.Library.openProfile(page.userId) }

            // ---- own: top artists this month
            Section {
                order: 0
                visible: page.ready && page.isMe && Backend.Library.profileTopArtists.count > 0
                Column {
                    spacing: Theme.space.xxs
                    SectionTitle { text: qsTr("Artistas más escuchados este mes") }
                    PrivateNote {}
                }
                Shelf {
                    id: artistList
                    objectName: "profileTopArtists"
                    model: Backend.Library.profileTopArtists
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
                        onActiveFocusChanged: { if (activeFocus) { artistList.currentIndex = index; viewport.revealCard(artistList, index) } }
                        Keys.onPressed: event => viewport.tabCard(artistList, event, index)
                        onClicked: page.navigate("artist", artistId, "")
                    }
                }
            }

            // ---- own: top tracks this month (Inicio's list)
            Section {
                order: 1
                visible: page.ready && page.isMe && Backend.Library.topTracks.count > 0
                Column {
                    spacing: Theme.space.xxs
                    SectionTitle { text: qsTr("Canciones más escuchadas este mes") }
                    PrivateNote {}
                }
                ListView {
                    id: topList
                    objectName: "profileTopTracks"
                    width: parent.width
                    height: Math.min(count, Theme.profile.topTracksShown) * Theme.size.trackRow
                    interactive: false
                    clip: true
                    reuseItems: true
                    model: Backend.Library.topTracks
                    delegate: TrackRow {
                        width: topList.width
                        visible: index < Theme.profile.topTracksShown
                        numberWidth: page.numberWidth
                        titleWidth: page.titleWidth
                        albumWidth: page.albumWidth
                        addedWidth: 0
                        keyboardCount: Math.min(topList.count, Theme.profile.topTracksShown)
                        sharedMenu: songMenu
                        onActiveFocusChanged: if (activeFocus) { topList.currentIndex = index; viewport.revealItem(this) }
                        playAction: i => Backend.Library.playTopTrack(i)
                        onSelected: rowIndex => topList.currentIndex = rowIndex
                        onAddToPlaylistRequested: uri => page.addToPlaylistRequested(uri)
                    }
                }
            }

            // ---- everyone's: public playlists (hidden when there is no data for them)
            Section {
                order: 2
                objectName: "profilePlaylistsSection"
                visible: page.ready && Backend.Library.profilePlaylists.count > 0
                SectionTitle { text: qsTr("Playlists públicas") }
                Shelf {
                    id: playlistList
                    objectName: "profilePlaylists"
                    visible: Backend.Library.profilePlaylists.count > 0
                    model: Backend.Library.profilePlaylists
                    delegate: MediaCard {
                        required property int index
                        required property string playlistId
                        required property string name
                        required property string owner
                        required property string imageUrl
                        required property int trackCount
                        required property var model
                        width: Theme.library.cardWidth
                        artSource: imageUrl
                        title: name
                        subtitle: trackCount === 1 ? qsTr("1 canción") : qsTr("%1 canciones").arg(trackCount)
                        uri: "spotify:playlist:" + playlistId
                        owned: model.owned   // gates the edit/delete entries of the card menu
                        onActiveFocusChanged: { if (activeFocus) { playlistList.currentIndex = index; viewport.revealCard(playlistList, index) } }
                        Keys.onPressed: event => viewport.tabCard(playlistList, event, index)
                        onClicked: page.navigate("playlist", playlistId, owner)
                    }
                }
            }
        }
    }
}
