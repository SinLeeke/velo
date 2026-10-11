import QtQuick
import QtQuick.Controls.impl
import QtQuick.Effects
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"
import "../../nav"
import ".."
import "../nowplaying"
import "creditsMemo.js" as CreditsMemo

// T5: «Reproduciendo» side panel of the library (as in Spotify's right column): the album (the
// backend has no playlist/context name) above a large cover, the title and artist with the heart,
// a LYRICS card in the cover's colour (the same LyricsView as Now Playing, smaller lines), the
// CREDITS of the song and the next song of the queue. A column that scrolls; no timer anywhere.
//
// The host (Library.qml) decides `shown` (wanted, something playing, room, queue closed) and
// reserves Theme.sidePanel.width for it. The panel docks like QueuePanel: same column, same gap
// to the window edge, so opening the queue simply takes its place. Motion: it fades in while
// sliding from the right (transform + opacity only); Theme.motion is 0 with Reducir movimiento,
// Modo rendimiento or the music paused, which makes it instant.
Item {
    id: root
    objectName: "nowPlayingSidePanel"
    property bool shown: false
    // «Siguiente en la cola» → «Abrir la cola»: the host opens the queue (which replaces this panel).
    signal queueRequested()

    readonly property string trackUri: Backend.Player.trackUri
    // T4: with another device playing, the card needs a duration (progress) to follow the song.
    readonly property bool lyricsAvailable: (Backend.Lyrics.status === "synced" || Backend.Lyrics.status === "plain")
        && (!Backend.Player.remote || Backend.Player.durationMs > 0)
    readonly property bool creditsAvailable: status === "ready" && contributors.length > 0
    readonly property bool nextAvailable: Backend.Player.queue.count > 0 && Backend.Player.queueStatus !== "unavailable"

    visible: shown || opacity > 0
    opacity: shown ? 1 : 0
    Behavior on opacity {
        OpacityAnimator {
            duration: Math.max(1, root.shown ? Theme.motion.slow : Theme.motion.fast)
            easing.type: Easing.Bezier
            easing.bezierCurve: root.shown ? Theme.motion.easeOut : Theme.motion.easeIn
        }
    }
    transform: Translate {
        x: root.shown ? 0 : root.width * Theme.entrance.queueShift
        Behavior on x {
            NumberAnimation {
                duration: root.shown ? Theme.motion.slow : Theme.motion.fast
                easing.type: Easing.Bezier
                easing.bezierCurve: root.shown ? Theme.motion.easeOut : Theme.motion.easeIn
            }
        }
    }
    implicitWidth: Theme.sidePanel.width

    // ---------------------------------------------------------------- motion helpers
    // Song-change crossfades and card fades run only with motion on and the panel on screen;
    // otherwise the change is instant (Modo rendimiento, Reducir movimiento, pause, minimized).
    function animatable() {
        const w = root.Window.window
        return Theme.motion.base > 0 && root.shown && root.visible && w !== null && w.visible && w.visibility !== Window.Minimized
    }
    // A-S2: lyrics / credits / next song arrive after the panel (asynchronous data) and would
    // pop in: they fade in (opacity only, `base`, easeOut). The height still changes at once (no
    // relayout animation); leaving is instant. While the panel itself is still entering
    // (opacity < 1) the cards come with it.
    function reveal(item, anim) {
        anim.stop()
        item.opacity = 1
        if (item.visible && animatable() && root.opacity >= 1) {
            item.opacity = 0
            anim.restart()
        }
    }
    OpacityAnimator { id: lyricsReveal; target: lyricsCard; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    OpacityAnimator { id: creditsReveal; target: creditsSection; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    OpacityAnimator { id: nextReveal; target: nextSection; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }

    // ---------------------------------------------------------------- credits (data of the dialog)
    // "loading" | "ready" | "none"; sections without data stay hidden (the failure state too).
    property string status: "none"
    property var contributors: []
    property string creditsUri: ""
    function roleLabel(role, title) {
        switch (role) {
        case "main_artist": return qsTr("Artista principal")
        case "featured_artist": return qsTr("Artista invitado")
        case "remixer": return qsTr("Remezcla")
        case "composer": return qsTr("Compositor")
        case "conductor": return qsTr("Director")
        case "orchestra": return qsTr("Orquesta")
        case "actor": return qsTr("Intérprete")
        case "songwriter": return qsTr("Compositor")
        case "producer": return qsTr("Producción")
        case "engineer": return qsTr("Ingeniería de sonido")
        case "mixing_engineer": return qsTr("Mezcla")
        case "mastering_engineer": return qsTr("Masterización")
        default: return title || qsTr("Artista")
        }
    }
    function loadCredits() {
        contributors = []
        creditsUri = ""
        if (!shown || trackUri === "" || Backend.Player.djSpeaking || CreditsMemo.isEmpty(trackUri)) {
            status = "none"
            return
        }
        creditsUri = trackUri
        status = "loading"
        Backend.Actions.requestCredits(trackUri)
    }
    function acceptCredits(uri, json) {
        if (uri !== creditsUri || uri === "")
            return
        let data = null
        try { data = JSON.parse(json) } catch (e) { data = null }
        if (!data || !Array.isArray(data.contributors)) { status = "none"; return }
        contributors = data.contributors.filter(c => c && c.name).map(c => ({ name: c.name, role: root.roleLabel(c.role, c.roleTitle) }))
        if (contributors.length === 0) CreditsMemo.markEmpty(uri)
        status = contributors.length > 0 ? "ready" : "none"
    }
    onShownChanged: if (shown) Qt.callLater(loadCredits); else { status = "none"; contributors = [] }
    onTrackUriChanged: if (shown) Qt.callLater(loadCredits)
    Connections {
        target: Backend.Actions
        function onCreditsReady(uri, json) { root.acceptCredits(uri, json) }
    }

    // ---------------------------------------------------------------- card
    RectangularShadow {
        anchors.fill: panel
        radius: Theme.radius.lg
        offset.y: Theme.elevation.raised.offsetY
        blur: Theme.elevation.raised.blur
        spread: Theme.elevation.raised.spread
        color: Qt.rgba(0, 0, 0, Theme.elevation.raised.alpha)
    }
    Rectangle {
        id: panel
        anchors.fill: parent
        anchors.rightMargin: Theme.queue.inset
        radius: Theme.radius.lg
        color: Theme.surface1
        border.width: Theme.stroke.hairline
        border.color: Theme.hairline
        clip: true

        Accessible.role: Accessible.Pane
        Accessible.name: qsTr("Reproduciendo")

        Flickable {
            id: flick
            objectName: "sidePanelFlick"
            anchors.fill: parent
            contentWidth: width
            contentHeight: column.implicitHeight + 2 * Theme.space.lg
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            T.ScrollBar.vertical: LibraryScrollBar {}

            Column {
                id: column
                x: Theme.space.lg
                y: Theme.space.lg
                width: flick.width - 2 * Theme.space.lg
                spacing: Theme.space.lg

                FadeLabel {
                    objectName: "sidePanelContext"
                    width: parent.width
                    // The playing list/context (Player.contextTitle, group B's Rust); the album until it arrives.
                    text: Backend.Player.contextTitle || Backend.Player.album || qsTr("Reproduciendo")
                    allow: root.shown
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.type.title
                    font.weight: Theme.type.bold
                    Accessible.role: Accessible.Heading
                }

                // A-S1: the cover crossfades like the one in the player bar (two tiles take turns;
                // the new art loads in the hidden one and fades in once decoded, so the slot never
                // flashes the placeholder). Opacity only, `base`, easeInOut.
                Item {
                    id: coverSlot
                    objectName: "sidePanelCover"
                    width: parent.width
                    height: width
                    property bool onA: true         // which tile shows the current cover
                    property bool pending: false    // the other tile is loading the next one
                    readonly property Item front: onA ? artA : artB
                    readonly property Item back: onA ? artB : artA
                    function nextCover() {
                        coverFade.stop()
                        front.opacity = 1
                        back.opacity = 0
                        back.source = Backend.Player.coverUrl
                        pending = true
                        Qt.callLater(showCover)
                    }
                    function showCover() {
                        if (!pending || back.status === Image.Loading)
                            return
                        pending = false
                        const incoming = back, outgoing = front
                        onA = !onA
                        if (!root.animatable()) {
                            incoming.opacity = 1
                            outgoing.opacity = 0
                            return
                        }
                        coverIn.target = incoming
                        coverOut.target = outgoing
                        coverFade.start()
                    }
                    Component.onCompleted: artA.source = Backend.Player.coverUrl
                    Connections {
                        target: Backend.Player
                        function onCoverUrlChanged() { coverSlot.nextCover() }
                    }
                    Connections {
                        target: coverSlot.back
                        function onStatusChanged() { coverSlot.showCover() }
                    }
                    LibraryArtwork { id: artA; anchors.fill: parent }
                    LibraryArtwork { id: artB; anchors.fill: parent; opacity: 0 }
                    ParallelAnimation {
                        id: coverFade
                        OpacityAnimator { id: coverIn; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                        OpacityAnimator { id: coverOut; to: 0; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                    }
                }

                Row {
                    width: parent.width
                    spacing: Theme.space.sm
                    Column {
                        width: parent.width - heart.width - Theme.space.sm
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.space.xxs
                        FadeLabel {
                            id: titleText
                            objectName: "sidePanelTitle"
                            width: parent.width
                            text: Backend.Player.title
                            allow: root.shown
                            underline: titleHover.hovered && Backend.Player.albumId !== ""
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.type.heading
                            font.weight: Theme.type.bold
                            Accessible.role: Accessible.StaticText
                            HoverHandler { id: titleHover; cursorShape: Backend.Player.albumId !== "" ? Qt.PointingHandCursor : Qt.ArrowCursor }
                            TapHandler { enabled: Backend.Player.albumId !== ""; onTapped: Nav.show("album", Backend.Player.albumId) }
                        }
                        FadeLabel {
                            objectName: "sidePanelArtist"
                            width: parent.width
                            text: Backend.Player.artist
                            allow: root.shown
                            underline: artistHover.hovered && Backend.Player.artistId !== ""
                            color: Theme.textSecondary
                            font.family: Theme.family
                            font.pixelSize: Theme.type.body
                            HoverHandler { id: artistHover; cursorShape: Backend.Player.artistId !== "" ? Qt.PointingHandCursor : Qt.ArrowCursor }
                            TapHandler { enabled: Backend.Player.artistId !== ""; onTapped: Nav.show("artist", Backend.Player.artistId) }
                        }
                    }
                    IconButton {
                        id: heart
                        objectName: "sidePanelHeart"
                        anchors.verticalCenter: parent.verticalCenter
                        text: Backend.Player.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Añadir a Tus me gusta")
                        iconSource: Backend.Player.liked ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : Qt.resolvedUrl("../../icons/favorite.svg")
                        ink: Backend.Player.liked ? Theme.accent : Theme.text
                        enabled: Backend.Player.hasSession
                        onClicked: { if (!Backend.Player.liked) pop(); Backend.Player.toggleLiked() }
                        // A-S3: a like that arrives from elsewhere (menu, search, bar) pops the heart too.
                        Connections {
                            target: Backend.Actions
                            function onSavedChanged(uri, saved) { if (saved && uri === root.trackUri) heart.pop() }
                        }
                    }
                }

                // ------------------------------------------------ lyrics card (the cover's colour)
                Rectangle {
                    id: lyricsCard
                    objectName: "sidePanelLyrics"
                    visible: root.lyricsAvailable
                    onVisibleChanged: root.reveal(lyricsCard, lyricsReveal)
                    width: parent.width
                    height: visible ? Theme.sidePanel.lyricsHeight : 0
                    radius: Theme.radius.lg
                    // The veil is the backdrop Rust computed the ink's contrast against.
                    color: Theme.cover.veil
                    Accessible.role: Accessible.Pane
                    Accessible.name: qsTr("Letra")
                    Text {
                        id: lyricsTitle
                        x: Theme.space.lg
                        y: Theme.space.md
                        text: qsTr("Letra")
                        color: Theme.cover.ink
                        font.family: Theme.family
                        font.pixelSize: Theme.type.body
                        font.weight: Theme.type.bold
                    }
                    Text {
                        anchors.right: parent.right
                        anchors.rightMargin: Theme.space.lg
                        anchors.verticalCenter: lyricsTitle.verticalCenter
                        visible: Backend.Lyrics.source !== ""
                        text: Backend.Lyrics.source === "spotify" ? qsTr("Letras de Spotify") : qsTr("Letras de LRCLIB")
                        color: Theme.coverInkLyric
                        font.family: Theme.family
                        font.pixelSize: Theme.type.caption
                    }
                    LyricsView {
                        id: lyricsView
                        objectName: "sidePanelLyricsView"
                        anchors.fill: parent
                        anchors.topMargin: lyricsTitle.y + lyricsTitle.height + Theme.space.xs
                        lyricSize: Theme.sidePanel.lyricSize
                        model: Backend.Lyrics.model
                        activeIndex: Backend.Lyrics.activeIndex
                        status: Backend.Lyrics.status
                        playing: Backend.Player.isPlaying
                        trackKey: Backend.Player.trackUri
                        plainText: Backend.Lyrics.plainText
                        onSeekRequested: ms => Backend.Player.seek(ms)
                    }
                }

                // ------------------------------------------------ credits
                Column {
                    id: creditsSection
                    objectName: "sidePanelCredits"
                    visible: root.creditsAvailable
                    onVisibleChanged: root.reveal(creditsSection, creditsReveal)
                    width: parent.width
                    spacing: Theme.space.sm
                    Item {
                        width: parent.width
                        height: Theme.type.title * 2
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Créditos")
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.type.title
                            font.weight: Theme.type.bold
                        }
                        SectionLink {
                            objectName: "sidePanelCreditsAll"
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Mostrar todo")
                            onClicked: Nav.showCredits(root.trackUri)
                        }
                    }
                    Repeater {
                        model: root.contributors.slice(0, Theme.sidePanel.creditsRows)
                        delegate: Column {
                            required property var modelData
                            width: creditsSection.width
                            spacing: Theme.space.xxs
                            Accessible.role: Accessible.ListItem
                            Accessible.name: qsTr("%1, %2").arg(modelData.name).arg(modelData.role)
                            Text { width: parent.width; text: modelData.name; elide: Text.ElideRight; color: Theme.text
                                font.family: Theme.family; font.pixelSize: Theme.type.body; font.weight: Theme.type.semibold }
                            Text { width: parent.width; text: modelData.role; elide: Text.ElideRight; color: Theme.textSecondary
                                font.family: Theme.family; font.pixelSize: Theme.type.caption }
                        }
                    }
                }

                // ------------------------------------------------ next in the queue
                Column {
                    id: nextSection
                    objectName: "sidePanelNext"
                    visible: root.nextAvailable
                    onVisibleChanged: root.reveal(nextSection, nextReveal)
                    width: parent.width
                    spacing: Theme.space.sm
                    Item {
                        width: parent.width
                        height: Theme.type.title * 2
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Siguiente en la cola")
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.type.title
                            font.weight: Theme.type.bold
                        }
                        SectionLink {
                            objectName: "sidePanelOpenQueue"
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            text: qsTr("Abrir la cola")
                            onClicked: root.queueRequested()
                        }
                    }
                    Row {
                        width: parent.width
                        spacing: Theme.space.md
                        readonly property var first: root.nextAvailable ? Backend.Player.queue.get(0) : null
                        Rectangle {
                            width: Theme.sidePanel.tile
                            height: width
                            radius: Theme.radius.sm
                            color: Theme.surface3
                            Image {
                                anchors.fill: parent
                                source: parent.parent.first ? parent.parent.first.imageUrl : ""
                                asynchronous: true
                                sourceSize: Qt.size(Theme.sidePanel.tile * 2, Theme.sidePanel.tile * 2)
                                fillMode: Image.PreserveAspectCrop
                                visible: status === Image.Ready
                            }
                        }
                        Column {
                            width: parent.width - Theme.sidePanel.tile - Theme.space.md
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: Theme.space.xxs
                            Text {
                                objectName: "sidePanelNextTitle"
                                width: parent.width
                                text: parent.parent.first ? parent.parent.first.title : ""
                                elide: Text.ElideRight
                                color: Theme.text
                                font.family: Theme.family
                                font.pixelSize: Theme.type.body
                                font.weight: Theme.type.semibold
                            }
                            Text {
                                width: parent.width
                                text: parent.parent.first ? parent.parent.first.artist : ""
                                elide: Text.ElideRight
                                color: Theme.textSecondary
                                font.family: Theme.family
                                font.pixelSize: Theme.type.caption
                            }
                        }
                    }
                }
            }
        }
    }

    // A-S1: a left-aligned line whose text crossfades on a song change (a ghost copy of the old
    // text fades out over the live one; opacity only, `base`, easeInOut, nothing slides). Same
    // pattern as LibraryPlayerBar.crossText(). `allow` is false while the panel is hidden, which
    // makes the change instant. Handlers declared inside act on the whole line.
    component FadeLabel: Item {
        id: fl
        property string text
        property string shownText: text
        property string ghostText: ""
        property bool allow: true
        property bool underline: false
        property color color
        property font font
        readonly property alias live: liveLabel
        readonly property alias ghost: ghostLabel
        implicitHeight: liveLabel.implicitHeight
        height: implicitHeight
        Accessible.name: fl.text
        onTextChanged: {
            if (fl.text === fl.shownText)
                return
            labelFade.stop()
            const old = fl.shownText
            fl.shownText = fl.text
            if (!fl.allow || !root.animatable() || old === "") {
                ghostLabel.opacity = 0
                liveLabel.opacity = 1
                return
            }
            fl.ghostText = old
            ghostLabel.opacity = 1
            liveLabel.opacity = 0
            labelFade.start()
        }
        Text {
            id: ghostLabel
            objectName: "ghost"
            width: parent.width
            text: fl.ghostText
            opacity: 0
            color: fl.color
            font.family: fl.font.family
            font.pixelSize: fl.font.pixelSize
            font.weight: fl.font.weight
            elide: Text.ElideRight
            maximumLineCount: 1
            Accessible.ignored: true
        }
        Text {
            id: liveLabel
            objectName: "live"
            width: parent.width
            text: fl.shownText
            color: fl.color
            font.family: fl.font.family
            font.pixelSize: fl.font.pixelSize
            font.weight: fl.font.weight
            font.underline: fl.underline
            elide: Text.ElideRight
            maximumLineCount: 1
            Accessible.ignored: true
        }
        ParallelAnimation {
            id: labelFade
            OpacityAnimator { target: ghostLabel; to: 0; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            OpacityAnimator { target: liveLabel; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        }
    }

    // Small text link of a section header (hover underline, keyboard focusable).
    component SectionLink: T.AbstractButton {
        id: link
        implicitWidth: label.implicitWidth + 2 * Theme.space.sm
        implicitHeight: Math.max(label.implicitHeight, Theme.size.minimumHitTarget)
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        padding: 0
        Accessible.name: text
        Accessible.role: Accessible.Button
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        contentItem: Text {
            id: label
            horizontalAlignment: Text.AlignRight
            verticalAlignment: Text.AlignVCenter
            text: link.text
            color: link.hovered ? Theme.text : Theme.textSecondary
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
            font.underline: link.hovered
        }
        background: Rectangle {
            radius: Theme.radius.sm
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: link.visualFocus ? Theme.focusRing : Theme.library.clearPaint
        }
    }
}
