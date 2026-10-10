import QtQuick
import ".."
import "../../theme"

// Top-right translucent pill: devices, lyrics, queue, theme, settings (ref_01's layout).
// The active tools (lyrics shown, queue open) are a solid pill with a dark glyph; both are
// exposed as toggles to accessibility. R2: the first slot was «Mini reproductor», which only
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
        activeFill: Theme.cover.control
        activeInk: Theme.cover.controlInk
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
