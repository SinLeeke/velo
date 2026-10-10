import QtQuick
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"

// Album / playlist / artist card. The art sits Theme.space.sm inside the hover surface, so
// lists of cards are offset by -Theme.space.sm to keep the art flush with page headings.
T.AbstractButton {
    id: card
    property string title
    property string subtitle
    property url artSource
    property bool portrait: false
    // M1: Spotify URI of what the card shows; enables the context menu (right click, Menu key).
    property string uri: ""
    property bool owned: false
    // R3 (P3-8): a play disc rises in over the art while the card is pointed at or focused, as
    // in the official app. Only for cards whose page can play them (`playable`); the page
    // decides what playing means (playRequested). The disc is not a Tab stop: Enter opens the
    // card and the context menu holds the rest.
    // playContext is the sole missing backend action (see the R3 integration request).
    // Hide the action until that invocable exists; other pages inherit the affordance.
    readonly property bool isTrack: uri.indexOf("spotify:track:") === 0
    property bool playable: uri !== "" && (isTrack || typeof Backend.Library.playContext === "function")
    signal playRequested()
    function play() {
        if (isTrack) Backend.Library.openLink(uri)
        else if (uri !== "" && playable) Backend.Library.playContext(uri)
        else playRequested()
    }
    readonly property bool canStart: Backend.Player.canControl || (Backend.Account.signedIn && Backend.Account.isPremium
        && !Backend.Account.audioReady && !Backend.Player.hasSession)
    readonly property var window: card.Window.window
    readonly property bool effectsAllowed: visible && Backend.Player.isPlaying && Theme.motion.scale > 0
        && (!window || (window.visible && window.visibility !== Window.Minimized
            && (window.active || !Theme.pauseEffectsInBackground)))
    readonly property bool playShown: playable && enabled && (hovered || visualFocus)
    implicitWidth: Theme.library.cardWidth
    // U4: as tall as its content plus the same Theme.space.sm inset as the top and the sides,
    // so the hover surface wraps the art and the text symmetrically.
    implicitHeight: cardContent.implicitHeight + Theme.space.sm
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    opacity: enabled ? 1 : Theme.library.disabledOpacity
    Accessible.name: qsTr("%1, %2").arg(title).arg(subtitle)
    // Space and Enter activate the focused card; the window's Space shortcut (play/pause) must not
    // take the key first (P1-3).
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()
    // Hover surface fades (opacity only): in at `instant`, out at `fast`.
    readonly property bool surfaceShown: enabled && (hovered || visualFocus)
    background: Rectangle {
        radius: Theme.radius.lg
        color: card.visualFocus ? Theme.accentSoft : Theme.surface2
        opacity: card.surfaceShown ? 1 : 0
        Behavior on opacity {
            OpacityAnimator { duration: Math.max(1, card.surfaceShown ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        border.width: card.visualFocus ? Theme.stroke.focus : 0
        border.color: Theme.focusRing
    }
    contentItem: Column {
        id: cardContent
        spacing: Theme.space.sm
        topPadding: Theme.space.sm
        LibraryArtwork {
            width: card.width - 2 * Theme.space.sm
            height: width
            anchors.horizontalCenter: parent.horizontalCenter
            source: card.artSource
            portrait: card.portrait
            fallback: card.portrait ? Qt.resolvedUrl("../../icons/person.svg") : Qt.resolvedUrl("../../icons/library_music.svg")
            // Fade + 8 px rise (opacity and Translate only), `fast`, ease-out; leaves the same way.
            Loader {
                active: card.playable
                x: parent.width - width - Theme.space.sm
                y: parent.height - height - Theme.space.sm
                width: Theme.library.cardPlay
                height: width
                sourceComponent: LibraryPlayButton {
                    objectName: "mediaCardPlay"
                    focusPolicy: Qt.NoFocus
                    iconSize: Theme.icon.lg
                    text: qsTr("Reproducir %1").arg(card.title)
                    iconSource: Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                    enabled: card.playShown && card.canStart
                    opacity: card.playShown ? (card.canStart ? 1 : Theme.library.disabledOpacity) : 0
                    Behavior on opacity {
                        enabled: card.effectsAllowed
                        OpacityAnimator { id: playFade; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                    transform: Translate {
                        y: card.playShown ? 0 : Theme.entrance.popoverShift
                        Behavior on y {
                            enabled: card.effectsAllowed
                            NumberAnimation { id: playRise; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                        }
                    }
                    Connections {
                        target: card
                        function onEffectsAllowedChanged() {
                            if (!card.effectsAllowed) { playFade.complete(); playRise.complete() }
                        }
                    }
                    onClicked: card.play()
                }
            }
        }
        Column {
            x: Theme.space.sm
            width: card.width - 2 * x
            spacing: Theme.space.xxs
            LibraryLabel { width: parent.width; text: card.title; font.weight: Theme.type.semibold }
            LibraryLabel { width: parent.width; text: card.subtitle; color: Theme.textSecondary; font.pixelSize: Theme.type.caption }
        }
    }

    function openMenu(x, y) {
        menuLoader.active = true
        menuLoader.item.popup(card, x, y)
    }
    TapHandler {
        acceptedButtons: Qt.RightButton
        enabled: card.uri !== ""
        onTapped: point => card.openMenu(point.position.x, point.position.y)
    }
    Keys.onMenuPressed: if (uri !== "") openMenu(width / 2, height / 2)
    // Created on first use: a page of cards does not build a menu per card.
    Loader {
        id: menuLoader
        active: false
        sourceComponent: MediaMenu { uri: card.uri; owned: card.owned }
    }
}
