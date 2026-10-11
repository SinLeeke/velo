import QtQuick
import Velo.Backend
import ".."
import "../../theme"

// Title (display, heavy, elided), artist + "more" button, album on a small soft line.
Column {
    id: root


    property string title
    property string artist
    property string album
    signal moreRequested()
    readonly property alias moreButton: more   // T4: NowPlaying keeps background clicks off it
    readonly property alias likeButton: heart  // B-1: the heart next to the artist
    // The heart reads Player.liked; a different surface saving the same song (menu, search, bar)
    // pops it too (see Connections), exactly like the player bar.
    property bool showHeart: true
    // Mixear (crossfade) for what is playing: only where it makes sense (playlists and Tus me gusta) and
    // when Velo's own engine can mix (gapless on, no DJ). Hidden otherwise rather than shown dead.
    readonly property bool mixShown: Player.mixAvailable && /^spotify:(playlist:|collection|user:[^:]+:collection)/.test(Player.contextUri)

    spacing: 0

    // A-N1: on a song change the three lines crossfade (a ghost copy of the old text fades out
    // over the live one fading in; opacity only, `base`, easeInOut, nothing slides), like the
    // cover and the library bar. Same pattern as LibraryPlayerBar.crossText(). The change is
    // instant with motion off (Modo rendimiento, Reducir movimiento, pause) or a hidden window.
    // The line keeps the live text's size, so the layout never moves; the ghost is drawn centred
    // over it with its own width (capped by `maxWidth`) and is never read by accessibility.
    component FadeText: Item {
        id: ft
        property string text
        property string shownText: text
        property string ghostText: ""
        property color color
        property font font
        property real maxWidth: width
        readonly property alias live: liveLabel
        readonly property alias ghost: ghostLabel
        readonly property alias fade: fadeAnim
        implicitWidth: liveLabel.implicitWidth
        implicitHeight: liveLabel.implicitHeight
        height: implicitHeight
        Accessible.name: ft.text
        function animatable() {
            const w = ft.Window.window
            return Theme.motion.base > 0 && ft.visible && w !== null && w.visible && w.visibility !== Window.Minimized
        }
        onTextChanged: {
            if (ft.text === ft.shownText)
                return
            fadeAnim.stop()
            const old = ft.shownText
            ft.shownText = ft.text
            if (!animatable() || old === "") {
                ghostLabel.opacity = 0
                liveLabel.opacity = 1
                return
            }
            ft.ghostText = old
            ghostLabel.opacity = 1
            liveLabel.opacity = 0
            fadeAnim.start()
        }
        Text {
            id: ghostLabel
            objectName: "ghost"
            anchors.horizontalCenter: parent.horizontalCenter
            width: Math.min(implicitWidth, ft.maxWidth)
            text: ft.ghostText
            opacity: 0
            color: ft.color
            font: ft.font
            elide: Text.ElideRight
            maximumLineCount: 1
            horizontalAlignment: Text.AlignHCenter
            Accessible.ignored: true
        }
        Text {
            id: liveLabel
            objectName: "live"
            anchors.horizontalCenter: parent.horizontalCenter
            width: Math.min(implicitWidth, ft.maxWidth)
            text: ft.shownText
            color: ft.color
            font: ft.font
            elide: Text.ElideRight
            maximumLineCount: 1
            horizontalAlignment: Text.AlignHCenter
            Accessible.ignored: true
        }
        ParallelAnimation {
            id: fadeAnim
            OpacityAnimator { target: ghostLabel; to: 0; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            OpacityAnimator { target: liveLabel; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        }
    }

    FadeText {
        objectName: "trackInfoTitle"
        width: root.width
        text: root.title
        color: Theme.cover.ink
        font.family: Theme.family
        font.pixelSize: Theme.type.display
        font.weight: Theme.type.heavy
        font.letterSpacing: Theme.type.tracking(Theme.type.display)
        Accessible.role: Accessible.Heading
    }

    Item { width: 1; height: Theme.space.sm }

    // Artist centered on its own text; the "more" button hangs off its right edge.
    Item {
        width: root.width
        height: Theme.size.iconButton - Theme.space.sm
        FadeText {
            id: artistText
            objectName: "trackInfoArtist"
            anchors.centerIn: parent
            maxWidth: root.width - 2 * (more.width + (root.showHeart ? heart.width + Theme.space.xs : 0) + (root.mixShown ? mixButton.width + Theme.space.xs : 0) + Theme.space.xs)
            width: Math.min(implicitWidth, maxWidth)
            text: root.artist
            color: Theme.coverInkSecondary
            font.family: Theme.family
            font.pixelSize: Theme.type.title
            font.weight: Theme.type.medium
        }
        // B-1: heart (Me gusta) to the left of "more". Saving pops it (1 -> likePop -> 1);
        // removing does not (rule 7: the glyph just changes). Ink: accent when saved.
        IconButton {
            id: heart
            objectName: "trackInfoHeart"
            visible: root.showHeart
            anchors.left: artistText.right
            anchors.leftMargin: Theme.space.xs
            anchors.verticalCenter: parent.verticalCenter
            width: parent.height
            height: parent.height
            text: Player.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Añadir a Tus me gusta")
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            Keys.onReturnPressed: clicked()
            Keys.onEnterPressed: clicked()
            NowPlayingToolTip { button: heart }
            enabled: Player.hasSession
            inkDisabled: Theme.cover.inkFaint
            iconSource: Player.liked ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : Qt.resolvedUrl("../../icons/favorite.svg")
            iconSize: Theme.icon.sm
            ink: Player.liked ? Theme.cover.ink : Theme.coverInkSecondary   // filled shape + strong ink (accent lacks contrast on the cover)
            hoverFill: Theme.cover.pillHover
            onClicked: { if (!Player.liked) pop(); else unpop(); Player.toggleLiked() }
        }
        Connections {
            target: Actions
            function onSavedChanged(uri, saved) { if (saved && uri === Player.trackUri) heart.pop() }
        }
        IconButton {
            id: mixButton
            objectName: "trackInfoMix"
            visible: root.mixShown
            anchors.left: root.showHeart ? heart.right : artistText.right
            anchors.leftMargin: Theme.space.xs
            anchors.verticalCenter: parent.verticalCenter
            width: parent.height
            height: parent.height
            text: qsTr("Mixear")   // stable name: as a toggle the screen reader already announces on/off
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            Keys.onReturnPressed: clicked()
            Keys.onEnterPressed: clicked()
            NowPlayingToolTip { button: mixButton }
            iconSource: Qt.resolvedUrl("../../icons/tune.svg")
            iconSize: Theme.icon.sm
            toggle: true
            active: Library.mixPlaying
            ink: Theme.coverInkSecondary
            activeInk: Theme.cover.ink
            hoverFill: Theme.cover.pillHover
            onClicked: Library.toggleMixPlaying()
        }
        IconButton {
            id: more
            anchors.left: root.mixShown ? mixButton.right : (root.showHeart ? heart.right : artistText.right)
            anchors.leftMargin: Theme.space.xs
            anchors.verticalCenter: parent.verticalCenter
            width: parent.height
            height: parent.height
            text: qsTr("Más opciones")
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            Keys.onReturnPressed: clicked()
            Keys.onEnterPressed: clicked()
            NowPlayingToolTip { button: more }
            iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
            iconSize: Theme.icon.sm
            ink: Theme.coverInkSecondary
            hoverFill: Theme.cover.pillHover
            onClicked: root.moreRequested()
        }
    }

    Item { width: 1; height: Theme.space.xxs }

    FadeText {
        objectName: "trackInfoAlbum"
        width: root.width
        text: root.album
        color: Theme.coverInkSecondary
        font.family: Theme.family
        font.pixelSize: Theme.type.caption
        font.weight: Theme.type.regular
    }
}
