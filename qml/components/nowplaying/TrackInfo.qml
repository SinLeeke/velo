import QtQuick
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

    spacing: 0

    Text {
        width: root.width
        text: root.title
        color: Theme.cover.ink
        font.family: Theme.family
        font.pixelSize: Theme.type.display
        font.weight: Theme.type.heavy
        font.letterSpacing: Theme.type.tracking(Theme.type.display)
        horizontalAlignment: Text.AlignHCenter
        elide: Text.ElideRight
        maximumLineCount: 1
        Accessible.role: Accessible.Heading
    }

    Item { width: 1; height: Theme.space.sm }

    // Artist centered on its own text; the "more" button hangs off its right edge.
    Item {
        width: root.width
        height: Theme.size.iconButton - Theme.space.sm
        Text {
            id: artistText
            anchors.centerIn: parent
            width: Math.min(implicitWidth, root.width - 2 * (more.width + Theme.space.xs))
            text: root.artist
            color: Theme.coverInkSecondary
            font.family: Theme.family
            font.pixelSize: Theme.type.title
            font.weight: Theme.type.medium
            elide: Text.ElideRight
            maximumLineCount: 1
        }
        IconButton {
            id: more
            anchors.left: artistText.right
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

    Text {
        width: root.width
        text: root.album
        color: Theme.coverInkSecondary
        font.family: Theme.family
        font.pixelSize: Theme.type.caption
        font.weight: Theme.type.regular
        horizontalAlignment: Text.AlignHCenter
        elide: Text.ElideRight
        maximumLineCount: 1
    }
}
