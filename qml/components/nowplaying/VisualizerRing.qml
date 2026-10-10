import QtQuick
import QtQuick.Shapes
import Velo.Backend
import "../../theme"
import "vizshape.js" as Viz

// Circular visualizer around the round cover (ref_05/06): "wave" is a soft blob whose radius
// follows the bands, "bars" are radial spokes. The round cover is painted on top, so the shapes
// start slightly inside `innerRadius` and only the part beyond the circle shows.
// Like VisualizerBand it only reads `bands` (only while running): no Timer and nothing at rest;
// bars use a Scale transform, the blob is one Shapes polygon. G4: the same display-rate glide as
// VisualizerBand when the bands arrive slower than the screen refreshes (off in Modo rendimiento).
// v0.9.2: `square` draws the same reactive around the square cover (rounded square of half side
// innerRadius, corners cornerRadius): spokes along the outline, perpendicular to it, and the blob
// pushed out along the outline's normal.
// R2: the blob and the glide blend are written into buffers this item owns (no JS allocation per
// frame), an unchanged frame leaves the path alone, and `gain` scales the levels without a copy.
Item {
    id: root

    property var bands: []
    property real gain: 1                  // multiplies every level (the volume the user hears)
    property string shape: "wave"          // "bars" | "wave"
    property color color: Theme.cover.glow
    property real innerRadius: 0           // radius of the round cover art (half side if square), px
    property bool square: false
    property real cornerRadius: 0          // square mode: the art's corner radius, px
    property bool interpolate: !Theme.performanceMode
    property bool running: Player.isPlaying
    readonly property bool animationOn: running && Player.isPlaying && visible && Window.window !== null
        && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || Window.active)
    onAnimationOnChanged: if (!animationOn && glider.running) { glider.stop(); glide = 1; drawn = bands }
    onInterpolateChanged: if (!interpolate && glider.running) { glider.stop(); glide = 1; drawn = bands }

    // ---- display-rate smoothing (G4; see VisualizerBand)
    readonly property real displayHz: Screen.refreshRate > 0 ? Screen.refreshRate : Theme.vizStyle.fallbackHz
    property var drawn: []
    property bool built: false
    Component.onCompleted: { built = true; drawn = bands; rebuild(true) }
    property var glideFrom: []             // own copy of where the glide starts (reused)
    property var glideBuf: []              // the blend on screen during a glide (reused)
    property real glide: 1
    property double lastBandsAt: 0
    onBandsChanged: {
        const now = Date.now(), gap = now - lastBandsAt
        const slow = lastBandsAt > 0 && gap > Theme.vizStyle.lerpRatio * 1000 / displayHz
        lastBandsAt = now
        glider.stop()
        if (!animationOn || !interpolate || !slow || !drawn || drawn.length !== bands.length) {
            glide = 1
            drawn = bands
            return
        }
        Viz.sameInto(glideFrom, drawn)
        if (Viz.equal(drawn, bands)) { drawn = bands; glide = 1; return }
        glider.duration = Math.min(gap, Theme.vizStyle.lerpMaxMs)
        glider.start()
    }
    onGlideChanged: if (glider.running) {
        drawn = Viz.mixInto(glideBuf, glideFrom, bands, glide)
        drawnChanged()
    }

    // ---- blob geometry, rebuilt per drawn set only when something changed (R2)
    property var blobPoints: []
    property var builtLevels: []
    function rebuild(force) {
        if (!built) return
        if (Viz.sameInto(builtLevels, drawn) && force !== true)
            return
        if (shape === "wave") {
            blobPoints = square
                ? Viz.squareInto(blobPoints, drawn, width / 2, height / 2, startRadius, outerRadius,
                                 Theme.vizStyle.ringSamples, Theme.vizStyle.blobFloor, gain, startCorner)
                : Viz.ringInto(blobPoints, drawn, width / 2, height / 2, startRadius, outerRadius,
                               Theme.vizStyle.ringSamples, Theme.vizStyle.blobFloor, gain)
            blobPath.path = blobPoints
        }
    }
    onDrawnChanged: rebuild(false)
    onWidthChanged: rebuild(true)
    onHeightChanged: rebuild(true)
    onGainChanged: rebuild(true)
    onShapeChanged: rebuild(true)
    onStartRadiusChanged: rebuild(true)
    onOuterRadiusChanged: rebuild(true)
    onSquareChanged: rebuild(true)
    onStartCornerChanged: rebuild(true)
    NumberAnimation {
        id: glider
        objectName: "ringGlider"
        target: root
        property: "glide"
        from: 0
        to: 1
        easing.type: Easing.Linear
        onFinished: root.drawn = root.bands
    }

    readonly property real outerRadius: Math.min(width, height) / 2
    // A shallow overlap hides gaps without swallowing quiet treble behind the cover.
    // Keep bass at the top and treble at the bottom; Rust adapts each real band.
    readonly property real startRadius: innerRadius * (1 - Theme.v1bViz.ringInsetRatio)
    readonly property real startCorner: cornerRadius * (1 - Theme.v1bViz.ringInsetRatio)

    readonly property color outerColor: Qt.rgba(color.r, color.g, color.b, Theme.vizStyle.alphaOuter)
    readonly property color innerColor: Qt.rgba(color.r, color.g, color.b, Theme.vizStyle.alphaInner)

    // ---- bars: 2 * bands spokes, mirrored left/right
    Repeater {
        model: root.shape === "bars" && !root.square ? 2 * Theme.viz.bands : 0
        delegate: Item {
            id: spoke
            required property int index
            x: root.width / 2
            y: root.height / 2
            rotation: index * 360 / (2 * Theme.viz.bands)
            Rectangle {
                id: stick
                x: -Theme.vizStyle.ringSpokeWidth / 2
                y: -root.outerRadius
                width: Theme.vizStyle.ringSpokeWidth
                height: root.outerRadius - root.startRadius
                radius: width / 2
                color: root.outerColor
                // Drawn at full length (startRadius..outerRadius) and scaled from its inner end:
                // a transform, so no relayout per update.
                transform: Scale {
                    origin.y: stick.height
                    yScale: Math.max(Theme.vizStyle.ringFloor,
                                     Viz.level(root.drawn[Viz.mirrorIndex(spoke.index, Theme.viz.bands)]) * root.gain)
                }
            }
        }
    }

    // ---- bars around the square: evenly along the outline, each perpendicular to it. The spot
    // (position and normal) depends only on the size, so it is computed once per resize.
    Repeater {
        model: root.shape === "bars" && root.square ? 2 * Theme.viz.bands : 0
        delegate: Item {
            id: edgeSpoke
            required property int index
            readonly property var spot: Viz.squareSpot({}, index / (2 * Theme.viz.bands), root.startRadius, root.startCorner)
            x: root.width / 2 + spot.x
            y: root.height / 2 + spot.y
            rotation: Math.atan2(spot.nx, -spot.ny) * 180 / Math.PI
            Rectangle {
                id: edgeStick
                x: -Theme.vizStyle.ringSpokeWidth / 2
                y: -height
                width: Theme.vizStyle.ringSpokeWidth
                height: root.outerRadius - root.startRadius
                radius: width / 2
                color: root.outerColor
                transform: Scale {
                    origin.y: edgeStick.height
                    yScale: Math.max(Theme.vizStyle.ringFloor,
                                     Viz.level(root.drawn[Viz.mirrorIndex(edgeSpoke.index, Theme.viz.bands)]) * root.gain)
                }
            }
        }
    }

    // ---- wave blob
    Shape {
        anchors.fill: parent
        visible: root.shape === "wave"
        preferredRendererType: Shape.GeometryRenderer
        ShapePath {
            strokeWidth: -1
            fillGradient: RadialGradient {
                centerX: root.width / 2; centerY: root.height / 2
                centerRadius: root.outerRadius
                focalX: centerX; focalY: centerY
                GradientStop { position: root.startRadius / root.outerRadius; color: root.innerColor }
                GradientStop { position: 1; color: root.outerColor }
            }
            PathPolyline { id: blobPath }
        }
    }
}
