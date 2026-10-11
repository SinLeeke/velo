import QtQuick
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"

// T3: «Reproduciendo en <dispositivo>» (as in Spotify): an accent strip under the player bar
// while ANOTHER device has the playback (Player.remote). It takes its height only while it is
// there (it fades in/out over Theme.motion.fast, opacity only, and gives the space back once
// invisible); with Reducir movimiento / Modo rendimiento / music paused the change is instant.
// No timer. The controls above already act on the active device (the backend does it).
Rectangle {
    id: strip
    objectName: "remoteDeviceStrip"
    readonly property bool shown: Backend.Player.remote && Backend.Player.hasSession
    // Height the layout reserves for it (the library lifts the player bar by this much).
    readonly property real reserve: visible ? Theme.remoteStrip.height : 0
    readonly property url deviceSource: Qt.resolvedUrl("../../icons/" + (["computer", "smartphone", "speaker", "tv"].indexOf(Backend.Player.deviceIcon) >= 0
                                                                          ? Backend.Player.deviceIcon : "devices") + ".svg")
    readonly property string label: qsTr("Reproduciendo en %1").arg(Backend.Player.deviceName)

    color: Theme.accent
    height: reserve
    opacity: shown ? 1 : 0
    visible: shown || opacity > 0
    Behavior on opacity {
        OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: shown ? Theme.motion.easeOut : Theme.motion.easeIn }
    }
    Accessible.role: Accessible.StaticText
    Accessible.name: label

    Row {
        anchors.centerIn: parent
        spacing: Theme.space.sm
        width: Math.min(implicitWidth, strip.width - 2 * Theme.space.lg)
        IconImage {
            anchors.verticalCenter: parent.verticalCenter
            source: strip.deviceSource
            sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
            color: Theme.accentInk
        }
        Text {
            objectName: "remoteDeviceText"
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, strip.width - 2 * Theme.space.lg - Theme.icon.sm - Theme.space.sm)
            text: strip.label
            elide: Text.ElideRight
            color: Theme.accentInk
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
        }
    }
}
