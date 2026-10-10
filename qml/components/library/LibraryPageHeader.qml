import QtQuick
import QtQuick.Controls.impl
import "../../theme"

// R4 (P2-2): the header shared by the playlist, album, «Tus me gusta», podcast, artist and
// profile pages, so the sibling pages keep one order and one scale: artwork, type, title,
// metadata (the page's own items, `meta: [ … ]`). The page puts its action row below it.
// The title uses type.hero and drops to type.display only when it would take more than
// Theme.library.heroTitleLines lines (measured once per title / width, no per-frame work).
Row {
    id: header
    property string kindLabel: ""
    property string title: ""
    property url artSource
    property url fallback: Qt.resolvedUrl("../../icons/library_music.svg")
    property bool portrait: false
    // «Tus me gusta»: the gradient tile with the heart, as in Inicio and the sidebar.
    property bool likedArt: false
    property alias artObjectName: artwork.objectName
    property alias titleObjectName: titleLabel.objectName
    property alias meta: metaColumn.data
    readonly property real artSize: Math.min(Theme.library.headerCover, Math.max(0, (width - spacing) / 2))
    readonly property real textWidth: Math.max(0, width - artSize - spacing)
    spacing: Theme.space.xl

    LibraryArtwork {
        id: artwork
        visible: !header.likedArt
        width: header.artSize
        height: width
        source: visible ? header.artSource : ""
        portrait: header.portrait
        fallback: header.fallback
    }
    Rectangle {
        visible: header.likedArt
        width: header.artSize
        height: width
        radius: Theme.radius.md
        gradient: Gradient {
            orientation: Gradient.Vertical
            GradientStop { position: 0; color: Theme.home.likedFrom }
            GradientStop { position: 1; color: Theme.home.likedTo }
        }
        IconImage {
            anchors.centerIn: parent
            source: Qt.resolvedUrl("../../icons/favorite_fill.svg")
            sourceSize: Qt.size(Theme.size.playDisc, Theme.size.playDisc)
            color: Theme.home.likedInk
        }
    }
    Column {
        width: header.textWidth
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.space.sm
        LibraryLabel {
            width: parent.width
            visible: text !== ""
            text: header.kindLabel
            color: Theme.textSecondary
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
        }
        TextMetrics {
            id: heroMetrics
            text: header.title
            font.family: Theme.family
            font.pixelSize: Theme.type.hero
            font.weight: Theme.type.bold
        }
        LibraryLabel {
            id: titleLabel
            readonly property int size: heroMetrics.advanceWidth <= header.textWidth * Theme.library.heroTitleLines ? Theme.type.hero : Theme.type.display
            width: parent.width
            visible: text !== ""
            text: header.title
            font.pixelSize: size
            font.letterSpacing: Theme.type.tracking(size)
            font.weight: Theme.type.bold
            wrapMode: Text.Wrap
            maximumLineCount: 2
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Column {
            id: metaColumn
            width: parent.width
            spacing: Theme.space.xs
        }
    }
}
