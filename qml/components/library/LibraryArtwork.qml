import QtQuick
import QtQuick.Shapes
import QtQuick.Controls.impl
import "../../theme"

// Artwork tile: rounded square (Theme.radius.md) or circle for artist portraits.
// Same technique as nowplaying/CoverArt: a Shape whose ShapePath.fillItem samples the Image
// texture (Qt 6.8+), so the rounded clip costs no shader, no layer and no offscreen pass.
// The Image decodes asynchronously at the tile's pixel size and is never drawn directly.
Item {
    id: art
    property url source
    property bool portrait: false
    property url fallback: Qt.resolvedUrl("../../icons/music_note.svg")
    readonly property real cornerRadius: portrait ? width / 2 : Theme.radius.md
    // Image.status of the art (S1-3: the player bar waits for Ready before crossfading).
    readonly property int status: cover.status
    function decodeSize(side) {
        const step = Theme.library.artDecodeStep
        return Math.max(step, Math.ceil(side / step) * step) * Screen.devicePixelRatio
    }
    Image {
        id: cover
        anchors.fill: parent
        visible: false
        source: art.source
        asynchronous: true
        // R3 (P2-9): decoded at the tile's size rounded up to a step (Theme.library.artDecodeStep),
        // so a grid that resizes continuously re-decodes only when a step is crossed; the GPU
        // scales the texture within a step (fillTransform below).
        sourceSize: Qt.size(art.decodeSize(art.width), art.decodeSize(art.height))
        fillMode: Image.PreserveAspectCrop
        smooth: true
    }
    // fillItem maps the texture at its decoded pixel size; scale it onto the tile.
    readonly property matrix4x4 textureFit: PlanarTransform.fromScale(
        width / Math.max(1, cover.implicitWidth), height / Math.max(1, cover.implicitHeight))
    Shape {
        anchors.fill: parent
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeWidth: -1
            fillColor: Theme.surface3
            PathRectangle { width: art.width; height: art.height; radius: art.cornerRadius }
        }
        ShapePath {
            strokeWidth: -1
            fillItem: cover.status === Image.Ready ? cover : null
            fillColor: Theme.library.clearPaint
            fillTransform: art.textureFit
            PathRectangle { width: art.width; height: art.height; radius: art.cornerRadius }
        }
    }
    IconImage {
        anchors.centerIn: parent
        visible: cover.status !== Image.Ready
        source: art.fallback
        sourceSize: Qt.size(Math.min(art.width / 2, Theme.size.playDisc), Math.min(art.height / 2, Theme.size.playDisc))
        color: Theme.textSecondary
    }
}
