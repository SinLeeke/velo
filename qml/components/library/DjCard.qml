import QtQuick
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"

// E5b: the DJ card of Inicio ("Hecho para ti" row). Same card as the shelves (MediaCard), with
// its own art: an accent tile with Livi's glyph. Click: start DJ Livi in Velo; during a session
// it is «Cambiar de onda». While Livi speaks the glyph breathes with its halo (DjHalo, same gates:
// still in pause, minimized, background pause and Modo rendimiento / Reducir movimiento).
MediaCard {
    id: card
    objectName: "homeDjCard"
    readonly property bool live: Backend.Player.djActive
    title: qsTr("DJ")
    subtitle: live ? qsTr("Cambiar de onda") : qsTr("Música elegida por Livi")
    onClicked: live ? Backend.Actions.djNextSegment() : Backend.Actions.startDj()

    // Drawn over MediaCard's (empty) artwork slot, same inset and corner radius.
    Rectangle {
        id: art
        z: 1
        x: Theme.space.sm
        y: Theme.space.sm
        width: card.width - 2 * Theme.space.sm
        height: width
        radius: Theme.radius.md
        color: Theme.accent
        // Hover lifts the tile a little (transform only): in at `fast`, back at `fast`.
        scale: card.hovered && card.enabled ? Theme.feel.hoverLift : 1
        Behavior on scale {
            ScaleAnimator { duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        DjHalo {
            id: halo
            anchors.centerIn: glyph
            width: Theme.dj.cardGlyph * Theme.dj.pulseScale
            height: width
            color: Theme.accentInk
        }
        IconImage {
            id: glyph
            anchors.centerIn: parent
            source: card.live ? Qt.resolvedUrl("../../icons/record_voice_over_fill.svg") : Qt.resolvedUrl("../../icons/record_voice_over.svg")
            sourceSize: Qt.size(Theme.dj.cardGlyph, Theme.dj.cardGlyph)
            color: Theme.accentInk
        }
        // The glyph swells with each breath of the halo (same duration, same gate).
        SequentialAnimation {
            id: swell
            running: halo.breathing
            loops: Animation.Infinite
            ScaleAnimator { target: glyph; from: 1; to: Theme.dj.glyphBreath; duration: Theme.dj.pulse; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            ScaleAnimator { target: glyph; from: Theme.dj.glyphBreath; to: 1; duration: Theme.dj.pulse; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            onRunningChanged: if (!running) Qt.callLater(card.settleGlyph)
        }
        LibraryLabel {
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.margins: Theme.space.md
            text: qsTr("DJ")
            color: Theme.accentInk
            font.pixelSize: Theme.type.heading
            font.weight: Theme.type.bold
        }
    }
    function settleGlyph() { if (!swell.running) glyph.scale = 1 }
}
