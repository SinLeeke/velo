import QtQuick
import ".."
import "../../theme"

// shuffle · previous · play/pause disc · next · repeat (repeat_one when repeatMode == 2).
// Active toggles get the soft pill behind them (ref_06); hover shows a pillHover disc.
Row {
    id: root

    property bool playing: false
    property bool shuffle: false
    property int repeatMode: 0
    property bool canControl: true

    signal playPauseClicked()
    signal previousClicked()
    signal nextClicked()
    signal shuffleClicked()
    signal repeatClicked()

    spacing: Theme.space.xl

    component TransportButton: IconButton {
        id: transportButton
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        NowPlayingToolTip { button: transportButton }
        anchors.verticalCenter: parent.verticalCenter
        enabled: root.canControl
        iconSize: Theme.icon.transport
        ink: Theme.cover.ink
        inkDisabled: Theme.cover.inkFaint
        hoverFill: Theme.cover.pillHover
        activeFill: Theme.cover.pill
        activeRadius: Theme.radius.md
    }

    TransportButton {
        text: qsTr("Aleatorio")
        iconSource: Qt.resolvedUrl("../../icons/shuffle.svg")
        active: root.shuffle
        Accessible.checkable: true      // a toggle: Accessible.checked follows `active`
        onClicked: root.shuffleClicked()
    }
    TransportButton {
        text: qsTr("Anterior")
        iconSource: Qt.resolvedUrl("../../icons/skip_previous.svg")
        onClicked: root.previousClicked()
    }
    PlayDisc {
        anchors.verticalCenter: parent.verticalCenter
        enabled: root.canControl
        playing: root.playing
        onClicked: root.playPauseClicked()
    }
    TransportButton {
        text: qsTr("Siguiente")
        iconSource: Qt.resolvedUrl("../../icons/skip_next.svg")
        onClicked: root.nextClicked()
    }
    TransportButton {
        text: root.repeatMode === 0 ? qsTr("Repetir")
            : root.repeatMode === 1 ? qsTr("Repetir canción") : qsTr("Desactivar repetición")
        iconSource: root.repeatMode === 2 ? Qt.resolvedUrl("../../icons/repeat_one.svg") : Qt.resolvedUrl("../../icons/repeat.svg")
        active: root.repeatMode > 0
        Accessible.checkable: true
        Accessible.description: root.repeatMode === 0 ? qsTr("Repetición desactivada")
                              : root.repeatMode === 1 ? qsTr("Repitiendo la lista") : qsTr("Repitiendo la canción")
        onClicked: root.repeatClicked()
    }
}
