import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import ".."

// S9/N3: minimal podcast page (Library.openShow → currentKind "show"). Header from the current*
// properties; episodes come in Library.tracks with its existing roles (title = episode,
// addedLabel = release date, durationMs, imageUrl). Double click or the play button plays.
Item {
    id: page
    property alias listView: episodes
    property string showId: ""
    property var playAction: index => Backend.Library.playIndex(index)
    property int requestedAtCount: -1
    property string localLoadError: ""
    readonly property string loadError: "loadError" in Backend.Library && Backend.Library.loadError !== ""
        ? Backend.Library.loadError : localLoadError
    readonly property bool failed: loadError !== "" && !Backend.Library.loading
    function reload() {
        localLoadError = ""
        requestedAtCount = -1
        if (episodes.count > 0) Backend.Library.fetchMore()
        else if (showId !== "") Backend.Library.openShow(showId)
    }
    Connections {
        target: Backend.Library
        function onLoadingChanged() { if (Backend.Library.loading) page.localLoadError = ""; else Qt.callLater(page.fetchNearEnd) }
        function onCurrentKindChanged() { page.localLoadError = ""; page.requestedAtCount = -1 }
    }
    Connections {
        target: Backend.Player
        function onError(message) { if (page.visible && Backend.Library.loading) page.localLoadError = message }
    }
    function durationText(ms) {
        const totalMinutes = Math.max(1, Math.round(ms / 60000))
        const hours = Math.floor(totalMinutes / 60), minutes = totalMinutes % 60
        return hours > 0 ? qsTr("%1 h %2 min").arg(hours).arg(minutes) : qsTr("%1 min").arg(minutes)
    }
    function fetchNearEnd() {
        if (visible && !failed && Backend.Library.hasMore && !Backend.Library.loading
                && episodes.contentY - episodes.originY + episodes.height >= episodes.contentHeight - Theme.size.trackRow * 4
                && requestedAtCount !== episodes.count) {
            requestedAtCount = episodes.count
            Backend.Library.fetchMore()
        }
    }
    ListView {
        id: episodes
        objectName: "podcastEpisodes"
        anchors.fill: parent
        clip: true
        reuseItems: true
        model: Backend.Library.currentKind === "show" ? Backend.Library.tracks : null
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        keyNavigationEnabled: true
        onContentYChanged: page.fetchNearEnd()
        onCountChanged: Qt.callLater(page.fetchNearEnd)
        header: Column {
            width: episodes.width - Theme.space.sm
            bottomPadding: Theme.space.xl
            LibraryPageHeader {
                width: parent.width
                kindLabel: qsTr("Pódcast")
                title: Backend.Library.currentTitle
                titleObjectName: "podcastTitle"
                artSource: Backend.Library.currentImageUrl
                fallback: Qt.resolvedUrl("../../icons/record_voice_over.svg")
                meta: [
                    LibraryLabel {
                        width: parent.width
                        text: Backend.Library.currentSubtitle
                        color: Theme.textSecondary
                        wrapMode: Text.Wrap
                    }
                ]
            }
        }
        delegate: LibraryButton {
            id: episode
            required property int index
            required property string uri
            required property string title
            required property string imageUrl
            required property string addedLabel
            required property int durationMs
            readonly property bool playingHere: uri !== "" && uri === Backend.Player.trackUri
            width: episodes.width - Theme.space.sm
            height: Theme.size.trackRow + Theme.space.lg
            leftPadding: Theme.space.sm
            rightPadding: Theme.size.iconButton + Theme.space.sm
            text: title
            Accessible.name: qsTr("%1, %2").arg(title).arg(addedLabel)
            function play() { page.playAction(index) }
            // A row selects on click; Enter/Space and double click play that episode.
            onClicked: episodes.currentIndex = index
            onDoubleClicked: play()
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            Keys.onReturnPressed: if (Backend.Player.canControl) play()
            Keys.onEnterPressed: if (Backend.Player.canControl) play()
            Keys.onSpacePressed: if (Backend.Player.canControl) play()
            onActiveFocusChanged: if (activeFocus) { episodes.currentIndex = index; episodes.positionViewAtIndex(index, ListView.Contain) }
            Keys.onPressed: event => {
                const backwards = event.key === Qt.Key_Backtab || (event.key === Qt.Key_Tab && (event.modifiers & Qt.ShiftModifier))
                const forwards = event.key === Qt.Key_Tab && !backwards
                if ((forwards && episodePlay.activeFocus) || (backwards && !episodePlay.activeFocus)) {
                    const next = index + (backwards ? -1 : 1)
                    if (next < 0 || next >= episodes.count) return
                    episodes.currentIndex = next
                    episodes.positionViewAtIndex(next, ListView.Contain)
                    episodes.forceLayout()
                    const item = episodes.itemAtIndex(next)
                    if (item) { item.forceActiveFocus(backwards ? Qt.BacktabFocusReason : Qt.TabFocusReason); event.accepted = true }
                }
            }
            contentItem: Row {
                spacing: Theme.space.md
                LibraryArtwork {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Theme.size.trackRow
                    height: width
                    source: episode.imageUrl !== "" ? episode.imageUrl : Backend.Library.currentImageUrl
                    fallback: Qt.resolvedUrl("../../icons/record_voice_over.svg")
                }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    width: episode.availableWidth - Theme.size.trackRow - parent.spacing
                    spacing: Theme.space.xs
                    LibraryLabel {
                        width: parent.width
                        text: episode.title
                        color: episode.playingHere ? Theme.accent : Theme.text
                        font.weight: Theme.type.semibold
                    }
                    LibraryLabel {
                        width: parent.width
                        text: episode.addedLabel !== "" ? qsTr("%1 · %2").arg(episode.addedLabel).arg(page.durationText(episode.durationMs))
                                                       : page.durationText(episode.durationMs)
                        color: Theme.textSecondary
                        font.pixelSize: Theme.type.caption
                        font.features: Theme.tabularFeatures
                    }
                }
            }
            IconButton {
                id: episodePlay
                objectName: "episodePlayButton"
                anchors.right: parent.right
                anchors.rightMargin: Theme.space.sm
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Reproducir %1").arg(episode.title)
                iconSource: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                ink: Theme.accent
                enabled: Backend.Player.canControl
                onClicked: episode.play()
            }
        }
        footer: Column {
            width: episodes.width
            topPadding: Theme.space.xl
            bottomPadding: Theme.space.xl
            spacing: Theme.space.md
            LibraryLabel {
                objectName: "podcastStatus"
                width: parent.width
                visible: Backend.Library.loading || page.failed || (episodes.count === 0 && Backend.Library.currentListable)
                text: Backend.Library.loading ? qsTr("Cargando episodios…") : page.failed ? page.loadError : qsTr("Todavía no hay episodios aquí.")
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
            LibraryButton { objectName: "podcastRetry"; visible: page.failed; text: qsTr("Reintentar"); enabled: page.showId !== "" || episodes.count > 0; onClicked: page.reload() }
        }
    }
}
