import QtQuick
import "../../theme"
import "../motion/morph.js" as Morph

// Lyrics (ref_03/ref_05), one component for every Lyrics.status:
//  - "synced": bold lines; the active one in cover.ink on a soft pill, parked at ~1/3 of the height
//    (gliding there on a line change); past and future lines in the dimmed ink; click/Enter on a line
//    asks to seek there (seekRequested(timeMs)). One Tab stop for the whole list (R2): Up/Down move
//    a keyboard cursor, Enter seeks. When the user scrolls (wheel, drag, keys) the view stops
//    following the song; it resumes Theme.nowPlaying.lyricsResumeMs after the last movement
//    (one-shot, never while paused or off screen), or at once on a click/Enter or a new song.
//  - "plain": unsynchronised text, scrollable.
//  - "loading", "none", "instrumental": a short headline plus a one-line explanation.
// Contrast: every text uses cover.ink or a dimmed cover.ink that stays >= 4.5:1 over the tinted
// background (cover.inkFaint, at 34 %, is for decoration only and is not used for text here).
Item {
    id: root


    property var model
    property int activeIndex: -1
    property string status: "none"
    property string plainText: ""
    // S1-NP: entrance progress of the lyrics (0..1, linear; NowPlaying maps it from `presence`).
    // Synced lines rise one after another, top line first; the other states rise as one block.
    property real intro: 1
    readonly property real introBlock: intro >= 1 ? 1 : Morph.ease(Theme.motion.npEase, intro)

    property bool playing: true
    property string trackKey: ""
    onTrackKeyChanged: resetFollowing()
    signal seekRequested(int timeMs)

    // ---- following the song (R2)
    property bool following: true
    readonly property bool onScreen: visible && Window.window !== null && Window.window.visible
                                     && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
    function userScrolled() {
        following = false
        scroll.stop()
        resume.stop()
    }
    function scheduleResume() {
        if (!following && playing && onScreen && !list.moving)
            resume.restart()
    }
    function follow(animated) {
        resume.stop()
        following = true
        list.park(animated, true)
    }
    function seekTo(timeMs) {
        resume.stop()
        following = true
        seekRequested(timeMs)
    }
    onPlayingChanged: {
        if (playing) scheduleResume()
        else { resume.stop(); scroll.stop() }
    }
    onOnScreenChanged: if (!onScreen && !following) follow(false)
    Timer {
        id: resume
        objectName: "lyricsResume"
        interval: Theme.nowPlaying.lyricsResumeMs
        onRunningChanged: if (running && (!root.playing || !root.onScreen)) stop()
        onTriggered: root.follow(true)
    }

    // Text column inset from the pill edge (the pill bleeds left of the text, as in ref_03).
    readonly property int inset: Theme.space.lg

    Accessible.role: Accessible.StaticText
    Accessible.name: status === "loading" ? qsTr("Buscando la letra…")
                   : status === "plain" ? qsTr("Letra sin sincronizar")
                   : status === "instrumental" ? qsTr("Instrumental")
                   : status === "none" ? qsTr("Sin letra")
                   : qsTr("Letra")

    // ------------------------------------------------------------ synced
    ListView {
        id: list
        objectName: "lyricsList"
        anchors.fill: parent
        visible: root.status === "synced"
        model: root.model
        reuseItems: true
        clip: true
        spacing: Theme.space.lg
        boundsBehavior: Flickable.StopAtBounds
        cacheBuffer: Math.max(0, height)
        // One Tab stop; the keyboard cursor starts on the line being sung.
        activeFocusOnTab: visible
        keyNavigationEnabled: true
        currentIndex: -1
        property bool keyboardNav: false
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Letra")
        onActiveFocusChanged: {
            if (!activeFocus) {
                keyboardNav = false
                return
            }
            keyboardNav = true
            if (currentIndex < 0 || root.following)
                currentIndex = Math.max(0, root.activeIndex)
            positionViewAtIndex(currentIndex, ListView.Contain)
        }
        onCurrentIndexChanged: if (activeFocus && keyboardNav) positionViewAtIndex(currentIndex, ListView.Contain)
        Keys.onPressed: event => {
            if (event.key === Qt.Key_Up || event.key === Qt.Key_Down
                    || event.key === Qt.Key_PageUp || event.key === Qt.Key_PageDown) {
                root.userScrolled()
                Qt.callLater(root.scheduleResume)
            }
        }
        Keys.onReturnPressed: if (currentItem) root.seekTo(currentItem.timeMs)
        Keys.onEnterPressed: if (currentItem) root.seekTo(currentItem.timeMs)
        Keys.onSpacePressed: if (currentItem) root.seekTo(currentItem.timeMs)
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }

        // Room above/below so the first and last lines can also sit at the 1/3 mark.
        header: Item { width: 1; height: list.height / 3 }
        footer: Item { width: 1; height: list.height * 2 / 3 }

        delegate: Item {
            id: line
            required property int index
            required property string text
            required property int timeMs
            readonly property bool active: index === root.activeIndex
            readonly property bool hot: hover.hovered && !active

            width: ListView.view.width
            height: label.implicitHeight + 2 * Theme.space.md
            Accessible.role: Accessible.ListItem
            Accessible.name: text
            Accessible.selected: active
            Accessible.onPressAction: root.seekTo(timeMs)

            // Lines cut by the top/bottom edge fade out instead of showing half glyphs.
            // A static function of position (opacity only, re-evaluated on scroll), not an animation.
            readonly property real edgeRoom: Math.min(y - list.contentY, list.height - (y - list.contentY + height))
            // S1-NP: during the entrance each line takes its turn by its height on screen.
            readonly property real introT: {
                if (root.intro >= 1)
                    return 1
                const stagger = Theme.motion.npLyricsStagger
                const start = Morph.clamp01((y - list.contentY) / Math.max(1, list.height)) * stagger
                return Morph.ease(Theme.motion.npEase, Morph.clamp01((root.intro - start) / (1 - stagger)))
            }
            opacity: Math.max(0, Math.min(1, 1 + 2 * edgeRoom / height)) * introT
            transform: Translate { y: (1 - line.introT) * Theme.motion.npLyricsRise }

            // Line handoff: the pill and the ink crossfade (opacity only) between the outgoing
            // and the incoming line, each driven by its own delegate's state change.
            Rectangle {
                anchors.fill: parent
                opacity: line.active ? 1 : 0     // at 0 the renderer skips it
                radius: Theme.radius.lg
                color: Theme.cover.pill
                Behavior on opacity {
                    enabled: root.playing && root.onScreen
                    OpacityAnimator { duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                }
            }
            Rectangle {   // keyboard cursor
                anchors.fill: parent
                visible: line.ListView.isCurrentItem && list.activeFocus && list.keyboardNav
                radius: Theme.radius.lg
                color: Theme.library.clearPaint
                border.width: Theme.stroke.focus
                border.color: Theme.cover.ink
            }
            Text {
                id: label
                x: root.inset
                width: parent.width - 2 * root.inset
                anchors.verticalCenter: parent.verticalCenter
                text: line.text
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                // Same ink as before (coverInkLyric is cover.ink at a lower alpha), expressed as
                // opacity so the handoff fades on the render thread instead of animating a color.
                color: Theme.cover.ink
                opacity: line.active || line.hot ? 1 : Theme.coverInkLyric.a
                Behavior on opacity {
                    enabled: root.playing && root.onScreen
                    OpacityAnimator { duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                }
                font.family: Theme.family
                font.pixelSize: Theme.type.lyric
                font.weight: Theme.type.bold
                font.letterSpacing: Theme.type.tracking(Theme.type.lyric)
                lineHeight: Theme.type.lyricLineHeight
            }
            HoverHandler {
                id: hover
                cursorShape: Qt.PointingHandCursor
            }
            TapHandler {
                onTapped: root.seekTo(line.timeMs)
            }
        }

        // Parks the active line at 1/3 of the height. A line change glides there (eased contentY,
        // which only moves the content node); a seek (more than lyricJumpLines away), a resize,
        // a new song or a hidden/minimized window jumps at once. One shot, retargeted from the
        // current position; a flick by the user takes over.
        property int parkedIndex: -1
        // `force`: glide even from far away (following resumes after the user scrolled).
        function park(animated, force) {
            if (root.status !== "synced" || height <= 0 || count === 0 || !root.following)
                return
            scroll.stop()
            const from = contentY
            if (root.activeIndex < 0) {
                positionViewAtBeginning()
            } else {
                positionViewAtIndex(root.activeIndex, ListView.Beginning)
                const item = itemAtIndex(root.activeIndex)
                const h = item ? item.height : 0
                contentY = contentY - (height / 3 - h / 2)
            }
            const to = contentY
            const near = force === true || parkedIndex >= 0 && root.activeIndex >= 0
                         && Math.abs(root.activeIndex - parkedIndex) <= Theme.feel.lyricJumpLines
            parkedIndex = root.activeIndex
            const w = list.Window.window
            const onScreen = list.visible && w !== null && w.visible && w.visibility !== Window.Minimized
            if (animated === true && root.playing && near && onScreen && Theme.motion.slow > 0 && Math.abs(to - from) >= 1) {
                contentY = from
                scroll.from = from
                scroll.to = to
                scroll.start()
            }
        }
        NumberAnimation {
            id: scroll
            target: list
            property: "contentY"
            duration: Theme.motion.slow
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.motion.easeInOut
        }
        onMovementStarted: root.userScrolled()
        onMovementEnded: root.scheduleResume()
        onHeightChanged: Qt.callLater(park)
        onCountChanged: Qt.callLater(park)
        Component.onCompleted: Qt.callLater(park)
    }

    Connections {
        target: root
        function onActiveIndexChanged() { list.park(true) }
        function onStatusChanged() { root.resetFollowing() }
    }
    function resetFollowing() {
        resume.stop()
        following = true
        list.parkedIndex = -1
        list.currentIndex = -1
        Qt.callLater(list.park)
    }
    Connections {
        target: root.model
        ignoreUnknownSignals: true
        function onModelReset() { root.resetFollowing() }
    }

    // ------------------------------------------------------------ plain (unsynchronised)
    Flickable {
        id: plainFlick
        anchors.fill: parent
        visible: root.status === "plain"
        opacity: root.introBlock
        transform: Translate { y: (1 - root.introBlock) * Theme.motion.npLyricsRise }
        clip: true
        contentWidth: width
        contentHeight: plainColumn.implicitHeight + 2 * Theme.space.xl
        boundsBehavior: Flickable.StopAtBounds
        activeFocusOnTab: true
        Accessible.role: Accessible.StaticText
        Accessible.name: qsTr("Letra sin sincronizar")
        Accessible.description: root.plainText

        // Keyboard scrolling (a line at a time, a page with PageUp/PageDown).
        function scrollBy(dy) {
            contentY = Math.max(0, Math.min(Math.max(0, contentHeight - height), contentY + dy))
        }
        Keys.onUpPressed: scrollBy(-Theme.type.heading * Theme.type.bodyLineHeight)
        Keys.onDownPressed: scrollBy(Theme.type.heading * Theme.type.bodyLineHeight)
        Keys.onPressed: event => {
            if (event.key === Qt.Key_PageUp) { scrollBy(-height); event.accepted = true }
            else if (event.key === Qt.Key_PageDown) { scrollBy(height); event.accepted = true }
        }

        Column {
            id: plainColumn
            x: root.inset
            y: Theme.space.xl
            width: parent.width - 2 * root.inset
            spacing: Theme.space.md

            Text {
                width: parent.width
                text: qsTr("Letra sin sincronizar")
                color: Theme.coverInkLyric
                font.family: Theme.family
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
            }
            Text {
                width: parent.width
                text: root.plainText
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                color: Theme.cover.ink
                font.family: Theme.family
                font.pixelSize: Theme.type.heading
                font.weight: Theme.type.semibold
                lineHeight: Theme.type.bodyLineHeight
            }
        }
    }

    // ------------------------------------------------------------ loading / none / instrumental
    Column {
        visible: root.status === "loading" || root.status === "none" || root.status === "instrumental"
        opacity: root.introBlock
        transform: Translate { y: (1 - root.introBlock) * Theme.motion.npLyricsRise }
        x: root.inset
        y: Math.round(root.height / 3 - height / 2)
        width: root.width - 2 * root.inset
        spacing: Theme.space.sm

        Text {
            width: parent.width
            text: root.status === "loading" ? qsTr("Buscando la letra…")
                : root.status === "instrumental" ? qsTr("Instrumental")
                : qsTr("Sin letra")
            wrapMode: Text.Wrap
            color: Theme.cover.ink
            font.family: Theme.family
            font.pixelSize: Theme.type.lyric
            font.weight: Theme.type.bold
            font.letterSpacing: Theme.type.tracking(Theme.type.lyric)
        }
        Text {
            width: parent.width
            text: root.status === "loading" ? qsTr("Un momento, la estamos pidiendo.")
                : root.status === "instrumental" ? qsTr("Esta pista no tiene letra. Solo música.")
                : qsTr("No encontramos la letra de esta canción.")
            wrapMode: Text.Wrap
            color: Theme.coverInkLyric
            font.family: Theme.family
            font.pixelSize: Theme.type.title
            font.weight: Theme.type.medium
        }
    }
}
