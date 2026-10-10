import QtQuick
import ".."
import "../../theme"

// Top-right translucent pill: devices, lyrics, queue, theme, settings (ref_01's layout).
// The active tools (devices open, lyrics shown, queue open) are white with a dark glyph, exposed
// as toggles to accessibility. v0.9.2: adjacent active tools merge into one white pill: each
// active slot has its own capsule and a bridge joins two active neighbours; capsules pop in
// (opacity + scale, Animators) and the bridge grows from the seam (xScale), so the join reads
// as a little fusion. Nothing runs at rest. R2: the first slot was «Mini reproductor», which only
// went back to the library (the close disc already does): it now opens the devices popover.
Rectangle {
    id: root

    property bool lyricsActive: true
    property bool queueActive: false
    property bool devicesActive: false
    property bool dark: true
    readonly property alias devicesButton: devices

    signal devicesClicked()
    signal lyricsClicked()
    signal queueClicked()
    signal themeClicked()
    signal settingsClicked()

    implicitWidth: row.implicitWidth + 2 * Theme.space.xs
    implicitHeight: Theme.size.toolPill
    radius: height / 2
    color: Theme.cover.pill

    component ToolButton: IconButton {
        id: tool
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        NowPlayingToolTip { button: tool }
        width: Theme.size.iconButton + Theme.space.xs
        height: Theme.size.toolPill - 2 * Theme.space.xs
        iconSize: Theme.icon.sm
        ink: Theme.cover.ink
        hoverFill: Theme.cover.pillHover
        activeFill: Theme.library.clearPaint   // drawn by `marks`, merged with the neighbours
        activeInk: Theme.cover.toolActiveInk
    }

    readonly property var actives: [devicesActive, lyricsActive, queueActive, false, false]
    Item {
        id: marks
        anchors.fill: row
        readonly property real slot: devices.width
        Repeater {
            model: root.actives.length
            delegate: Rectangle {
                id: capsule
                required property int index
                readonly property bool on: root.actives[index]
                objectName: "toolMark" + index
                x: index * marks.slot
                width: marks.slot
                height: marks.height
                radius: height / 2
                color: Theme.cover.toolActive
                opacity: on ? 1 : 0
                scale: on ? 1 : Theme.feel.toolMarkFrom
                Behavior on opacity { OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
                Behavior on scale { ScaleAnimator { duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
            }
        }
        Repeater {
            model: root.actives.length - 1
            delegate: Rectangle {
                id: bridge
                required property int index
                readonly property bool on: root.actives[index] && root.actives[index + 1]
                objectName: "toolBridge" + index
                x: (index + 0.5) * marks.slot
                width: marks.slot
                height: marks.height
                color: Theme.cover.toolActive
                opacity: on ? 1 : 0
                transform: Scale {
                    id: seam
                    origin.x: bridge.width / 2
                    xScale: bridge.on ? 1 : 0
                    Behavior on xScale { NumberAnimation { duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
                }
                Behavior on opacity { OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
            }
        }
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 0

        ToolButton {
            id: devices
            objectName: "npDevicesButton"
            text: qsTr("Dispositivos")
            iconSource: Qt.resolvedUrl("../../icons/devices.svg")
            active: root.devicesActive
            Accessible.checkable: true
            onClicked: root.devicesClicked()
        }
        ToolButton {
            objectName: "npLyricsButton"
            text: qsTr("Letra")
            iconSource: Qt.resolvedUrl("../../icons/lyrics.svg")
            active: root.lyricsActive
            Accessible.checkable: true
            onClicked: root.lyricsClicked()
        }
        ToolButton {
            objectName: "npQueueButton"
            text: qsTr("Cola")
            iconSource: Qt.resolvedUrl("../../icons/queue_music.svg")
            active: root.queueActive
            Accessible.checkable: true
            onClicked: root.queueClicked()
        }
        ToolButton {
            text: root.dark ? qsTr("Cambiar a modo claro") : qsTr("Cambiar a modo oscuro")
            iconSource: root.dark ? Qt.resolvedUrl("../../icons/light_mode.svg") : Qt.resolvedUrl("../../icons/dark_mode.svg")
            onClicked: root.themeClicked()
        }
        ToolButton {
            text: qsTr("Ajustes")
            // No hay settings.svg; tune.svg es el icono de ajustes disponible.
            iconSource: Qt.resolvedUrl("../../icons/tune.svg")
            onClicked: root.settingsClicked()
        }
    }
}
