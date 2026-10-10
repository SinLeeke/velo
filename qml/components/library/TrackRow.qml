import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import ".."
import "../../nav"

FocusScope {
    id: row
    required property int index
    required property string uri
    required property string title
    required property string artist
    required property string album
    required property int durationMs
    required property string imageUrl
    required property bool liked
    required property string addedLabel
    property string playlistId: ""
    property bool ownPlaylist: false
    // M1: "Ir al artista / álbum" (roles from the tracks model when it provides them).
    required property string artistId
    required property string albumId
    // Fixed "#" column width, measured once by the page (TextMetrics), identical for every row.
    property real numberWidth: Theme.space.xxxl
    // F3: column widths, computed once per page width by TrackListPage (0 hides a column).
    property real titleWidth: 0
    property real albumWidth: 0
    property real addedWidth: 0
    // F4 "Ver como: Compacta": no artwork, title and artist on one line, shorter row.
    property bool compact: false
    // R4: the page's one TrackMenu, shared by all its rows (filled with this row's song when it
    // opens). A page that passes none gets one built on first use for this row only.
    property TrackMenu sharedMenu: null
    readonly property TrackMenu contextMenu: sharedMenu ? sharedMenu : (ownMenu.item as TrackMenu)
    signal addToPlaylistRequested(string uri)
    signal selected(int rowIndex)
    implicitHeight: compact ? Theme.trackList.compactRow : Theme.size.trackRow
    activeFocusOnTab: true
    Accessible.role: Accessible.ListItem
    Accessible.name: qsTr("%1, %2, %3").arg(title).arg(artist).arg(album)
    // Lists that are not Library.tracks (artist "Populares") pass their own play function.
    property var playAction: null
    property int keyboardCount: ListView.view ? ListView.view.count : 0
    function play() { if (playAction) playAction(index); else Backend.Library.playIndex(index) }
    function focusFirst(reason) { more.focus = false; forceActiveFocus(reason) }
    function focusLast() { more.forceActiveFocus(Qt.BacktabFocusReason) }
    function openMenu() {
        if (!sharedMenu) ownMenu.active = true
        contextMenu.openFor(row, width - contextMenu.width, height)
    }
    ListView.onPooled: if (contextMenu && contextMenu.row === row) contextMenu.close()
    onActiveFocusChanged: {
        if (activeFocus && ListView.view) {
            ListView.view.currentIndex = index
            ListView.view.positionViewAtIndex(index, ListView.Contain)
        }
    }
    // P1-3: Space on a focused row plays it; the window's Space shortcut (play/pause) must not
    // take the key first.
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space && event.modifiers === Qt.NoModifier) event.accepted = true }
    Keys.onSpacePressed: play()
    Keys.onReturnPressed: play()
    Keys.onEnterPressed: play()
    Keys.onMenuPressed: openMenu()
    Keys.onPressed: event => handleKey(event)
    function handleKey(event) {
        if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
            openMenu(); event.accepted = true
        } else if (ListView.view) {
            const backwards = event.key === Qt.Key_Backtab || (event.key === Qt.Key_Tab && (event.modifiers & Qt.ShiftModifier))
            const forwards = event.key === Qt.Key_Tab && !backwards
            if (forwards && !more.activeFocus) {
                more.forceActiveFocus(Qt.TabFocusReason)
                event.accepted = true
                return
            }
            if (backwards && more.activeFocus) {
                focusFirst(Qt.BacktabFocusReason)
                event.accepted = true
                return
            }
            // Let Tab visit the row's "…" first; at its edge create/reveal the next delegate.
            if ((forwards && more.activeFocus) || (backwards && !more.activeFocus)) {
                const next = index + (backwards ? -1 : 1)
                if (next >= 0 && next < keyboardCount) {
                    const view = ListView.view
                    view.currentIndex = next
                    view.positionViewAtIndex(next, ListView.Contain)
                    view.forceLayout()
                    const item = view.itemAtIndex(next) as TrackRow
                    if (item) {
                        if (backwards) item.focusLast()
                        else item.focusFirst(Qt.TabFocusReason)
                        event.accepted = true
                    }
                }
            }
        }
    }
    // Hover / focus surface fades (opacity only): in at `instant`, out at `fast`.
    readonly property bool surfaceShown: pointer.containsMouse || activeFocus
    Rectangle {
        anchors.fill: parent
        radius: Theme.radius.md
        color: row.activeFocus ? Theme.accentSoft : Theme.surface2
        opacity: row.surfaceShown ? 1 : 0
        Behavior on opacity {
            OpacityAnimator { duration: Math.max(1, row.surfaceShown ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        border.width: row.activeFocus ? Theme.stroke.focus : 0
        border.color: Theme.focusRing
    }
    // C7: the row of the song that is playing swaps its number for three accent bars. While the
    // visualizer runs they follow its bands (the screen clock, no timer of their own; only this
    // row reads them); otherwise they rest at fixed heights. Scale transforms only.
    readonly property bool playingHere: uri !== "" && uri === Backend.Player.trackUri
    // S9 (4b): only while this row can be seen. Under Now Playing the library is hidden (and the
    // Visualizer keeps publishing for the big band): `visible` is the effective visibility, so the
    // bars stop re-evaluating there too. Paused or minimized: not playing / not running.
    readonly property var levels: playingHere && visible && Backend.Player.isPlaying && Backend.Visualizer.running ? Backend.Visualizer.bands : null
    // Reused delegates: whatever an exit/remove transition left behind is reset here.
    ListView.onReused: opacity = 1
    MouseArea {
        id: pointer
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onClicked: mouse => {
            row.forceActiveFocus(); row.selected(row.index)
            if (mouse.button === Qt.RightButton) row.openMenu()
        }
        onDoubleClicked: mouse => { if (mouse.button === Qt.LeftButton) row.play() }
    }
    Row {
        anchors.fill: parent
        anchors.topMargin: Theme.space.sm
        anchors.bottomMargin: Theme.space.sm
        anchors.leftMargin: Theme.trackList.rowInset
        anchors.rightMargin: Theme.trackList.rowInset
        spacing: Theme.space.md
        Item {
            // 0 = no "#" column (search "Canciones"): hidden, so it takes no spacing either.
            visible: row.numberWidth > 0
            width: row.numberWidth
            height: number.implicitHeight
            anchors.verticalCenter: parent.verticalCenter
            LibraryLabel {
                id: number
                width: parent.width
                text: qsTr("%1").arg(row.index + 1)
                color: Theme.textTertiary
                horizontalAlignment: Text.AlignRight
                elide: Text.ElideNone
                font.features: Theme.tabularFeatures
                visible: !row.playingHere
            }
            Loader {   // created only in the playing row: the other delegates stay light
                active: row.playingHere
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                sourceComponent: nowBars
            }
        }
        Component {
            id: nowBars
            Row {
                height: Theme.icon.sm
                spacing: Theme.space.xxs
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Sonando ahora")
                Repeater {
                    model: Theme.feel.nowPlayingBands
                    Rectangle {
                        required property int index
                        required property int modelData
                        width: Theme.stroke.progress
                        height: Theme.icon.sm
                        radius: width / 2
                        color: Theme.accent
                        transform: Scale {
                            origin.y: Theme.icon.sm
                            yScale: row.levels ? Math.max(Theme.feel.nowPlayingFloor, Math.min(1, row.levels[modelData] || 0))
                                               : Theme.feel.nowPlayingRest[index]
                        }
                    }
                }
            }
        }
        LibraryArtwork {
            visible: !row.compact
            width: Theme.trackList.rowArt
            height: width
            anchors.verticalCenter: parent.verticalCenter
            source: row.compact ? "" : row.imageUrl
        }
        Column {
            visible: !row.compact
            width: row.titleWidth
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.xxs
            LibraryLabel { width: parent.width; text: row.title; font.weight: Theme.type.medium }
            LibraryLabel { width: parent.width; text: row.artist; color: Theme.textSecondary; font.pixelSize: Theme.type.caption }
        }
        Row {
            visible: row.compact
            width: row.titleWidth
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.sm
            LibraryLabel {
                id: compactTitle
                width: Math.min(implicitWidth, parent.width * Theme.trackList.compactTitleShare)
                anchors.baseline: compactArtist.baseline
                text: row.title
                font.weight: Theme.type.medium
            }
            LibraryLabel {
                id: compactArtist
                width: parent.width - compactTitle.width - parent.spacing
                anchors.verticalCenter: parent.verticalCenter
                text: row.artist
                color: Theme.textSecondary
                font.pixelSize: Theme.type.caption
            }
        }
        LibraryLabel {
            id: albumLabel
            width: row.albumWidth
            visible: width > 0
            anchors.verticalCenter: parent.verticalCenter
            text: row.album
            color: Theme.textSecondary
            font.pixelSize: Theme.type.caption
        }
        LibraryLabel {
            width: row.addedWidth
            visible: width > 0
            anchors.verticalCenter: parent.verticalCenter
            text: row.addedLabel
            color: Theme.textSecondary
            font.pixelSize: Theme.type.caption
        }
        // P2-4: the heart saves / removes the song (Actions.setSaved). A saved song always shows
        // the filled accent heart; the empty one appears only while the row is hovered or
        // focused (opacity, no relayout). Mouse only: the row's menu has the same action for
        // the keyboard, so the row keeps two Tab stops (row and "…").
        Item {
            width: Theme.icon.sm
            height: Theme.size.iconButton
            anchors.verticalCenter: parent.verticalCenter
            IconButton {
                id: likeButton
                objectName: "trackLikeButton"
                anchors.centerIn: parent
                focusPolicy: Qt.NoFocus
                text: row.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Guardar en Tus me gusta")
                iconSource: row.liked ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : Qt.resolvedUrl("../../icons/favorite.svg")
                iconSize: Theme.icon.sm
                ink: Theme.textSecondary
                active: row.liked
                Accessible.checkable: true
                activeFill: Theme.library.clearPaint
                activeInk: Theme.accent
                opacity: row.liked || row.surfaceShown || hovered ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                onClicked: Backend.Actions.setSaved(row.uri, !row.liked)
            }
        }
        LibraryLabel {
            id: duration
            width: Theme.trackList.durationWidth
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("%1:%2").arg(Math.floor(row.durationMs / 60000)).arg(("0" + Math.floor(row.durationMs / 1000) % 60).slice(-2))
            color: Theme.textSecondary
            font.pixelSize: Theme.type.caption
            font.features: Theme.tabularFeatures
            horizontalAlignment: Text.AlignRight
        }
        IconButton {
            id: more
            objectName: "trackMoreButton"
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Opciones de %1").arg(row.title)
            iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
            Keys.onPressed: event => row.handleKey(event)
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            onClicked: row.openMenu()
        }
    }
    // Fallback menu (pages that pass no sharedMenu): built on first use, for this row only.
    Loader {
        id: ownMenu
        active: false
        sourceComponent: TrackMenu {}
    }
    Connections {
        target: Backend.Actions
        function onSavedChanged(uri, saved) {
            if (uri !== row.uri)
                return
            if (saved && !row.liked)
                likeButton.pop()
            row.liked = saved
        }
    }
}
