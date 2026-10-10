import QtQuick
import QtQuick.Shapes
import Velo.Backend
import "../../theme"
import "vizshape.js" as Viz

// Horizontal visualizer band (ref_03): "wave" (filled soft hills) or "bars" (48 columns).
// It only reads `bands`, which the backend republishes only while the visualizer is running;
// there is no Timer and no per-frame binding at rest, so with the music paused (or the window
// minimized) nothing is re-evaluated and the scene graph is idle.
//  - bars: one Rectangle per band, drawn at full height and scaled with a Scale transform from the
//    bottom edge (a transform, so no relayout and no geometry change per update).
//  - wave: a single QtQuick.Shapes polygon (GeometryRenderer) rebuilt once
//    per drawn band set from ~190 points. No ShaderEffect.
// R2 (per frame, at the screen's rate): the polygon, the glide blend and the thin bars are written
// into buffers this item owns (vizshape *Into), so a frame allocates nothing in JS; a frame whose
// levels did not change (silence, a repeated set) does not touch the path at all; `gain` (the
// volume curve, from NowPlaying) is applied while reading the bands, with no copy.
// G4 (ronda 2):
//  - Contained: every shape stays inside the item. Levels bend softly toward the top edge
//    (Theme.vizStyle.knee) and the wave/bars leave room for their own stroke, so nothing pokes out
//    of the player bar.
//  - Display rate: when the bands arrive more than `lerpRatio` screen frames apart, the drawing
//    glides from the set on screen to the new one over the measured gap (one short NumberAnimation
//    of `glide`, capped at `lerpMaxMs`). Each step is one 48-value blend and one path rebuild; it
//    ends before the next update and never runs without new bands. Off in Modo rendimiento.
Item {
    id: root

    property var bands: []
    property real gain: 1                  // multiplies every level (the volume the user hears)
    property string shape: "wave"          // "bars" | "wave"
    property color color: Theme.cover.glow
    implicitHeight: Theme.v1Viz.bandMaxHeight
    // S9/N1 (player bar): `barCount` > 0 draws that many thin bars (one stroked PathMultiline in
    // a single Shape, resampled from the bands) instead of one Rectangle per band; `outline`
    // adds a stroke along the crest of the wave. Defaults keep the Now Playing band as it was.
    property int barCount: 0
    property real barWidth: Theme.stroke.progress
    property bool outline: false
    property real crestAlpha: Theme.vizStyle.alphaTop
    property real baseAlpha: Theme.vizStyle.alphaBottom
    // G4: glide between band sets that arrive slower than the screen refreshes.
    property bool interpolate: !Theme.performanceMode
    property bool running: Player.isPlaying
    readonly property bool animationOn: running && Player.isPlaying && visible && Window.window !== null
        && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || Window.active)
    onAnimationOnChanged: if (!animationOn && glider.running) { glider.stop(); glide = 1; drawn = bands }
    onInterpolateChanged: if (!interpolate && glider.running) { glider.stop(); glide = 1; drawn = bands }

    readonly property color crest: Qt.rgba(color.r, color.g, color.b, crestAlpha)
    readonly property color base: Qt.rgba(color.r, color.g, color.b, baseAlpha)
    readonly property bool thinBars: shape === "bars" && barCount > 0
    readonly property real outlineWidth: Theme.stroke.hairline * 2

    // ---- display-rate smoothing (G4)
    readonly property real displayHz: Screen.refreshRate > 0 ? Screen.refreshRate : Theme.vizStyle.fallbackHz
    property var drawn: []                 // the band set on screen (everything below draws it)
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
        if (Viz.equal(drawn, bands)) { drawn = bands; glide = 1; return }
        Viz.sameInto(glideFrom, drawn)
        glider.duration = Math.min(gap, Theme.vizStyle.lerpMaxMs)
        glider.start()
    }
    onGlideChanged: if (glider.running) {
        drawn = Viz.mixInto(glideBuf, glideFrom, bands, glide)
        drawnChanged() // var buffers keep their identity; notify transform bindings explicitly
    }

    // ---- geometry, rebuilt per drawn set only when something changed (R2)
    property var wavePoints: []            // the wave polygon (a reused buffer)
    property var crestPoints: []           // its top edge, for the outline (reused)
    property var spectrumLines: []         // thin bars (reused)
    property var builtLevels: []
    function rebuild(force) {
        if (!built) return
        const same = Viz.sameInto(builtLevels, drawn)
        if (same && force !== true)
            return
        if (shape === "wave") {
            wavePoints = Viz.waveInto(wavePoints, drawn, width, height, Theme.vizStyle.waveSubdivisions,
                                      Theme.vizStyle.floor, Theme.vizStyle.waveSmoothing,
                                      outline ? outlineWidth : 0, Theme.vizStyle.knee, gain)
            wavePath.path = wavePoints
            if (outline)
                crestPoints = Viz.crestInto(crestPoints, wavePoints)
            crestPath.path = crestPoints
        }
        if (thinBars) {
            spectrumLines = Viz.spectrumInto(spectrumLines, drawn, width, height, barCount,
                                             Theme.vizStyle.floor, Theme.vizStyle.waveSmoothing, barWidth / 2,
                                             Theme.vizStyle.knee, gain)
            barsPath.paths = spectrumLines
        }
    }
    onDrawnChanged: rebuild(false)
    onWidthChanged: rebuild(true)
    onHeightChanged: rebuild(true)
    onGainChanged: rebuild(true)
    onShapeChanged: rebuild(true)
    onOutlineChanged: rebuild(true)
    onBarCountChanged: rebuild(true)
    onBarWidthChanged: rebuild(true)
    NumberAnimation {
        id: glider
        objectName: "bandGlider"
        target: root
        property: "glide"
        from: 0
        to: 1
        easing.type: Easing.Linear
        onFinished: root.drawn = root.bands
    }

    // ---- bars
    Repeater {
        model: root.shape === "bars" && !root.thinBars ? Theme.viz.bands : 0
        delegate: Rectangle {
            id: bar
            required property int index
            readonly property real barWidth: (root.width - (Theme.viz.bands - 1) * Theme.vizStyle.barGap) / Theme.viz.bands
            x: index * (barWidth + Theme.vizStyle.barGap)
            y: 0
            width: barWidth
            height: root.height
            radius: Math.min(barWidth / 2, Theme.radius.xs)
            gradient: Gradient {
                GradientStop { position: 0; color: root.crest }
                GradientStop { position: 1; color: root.base }
            }
            transform: Scale {
                origin.y: bar.height
                yScale: Math.min(1, Math.max(Theme.vizStyle.floor,
                                             Viz.limit(Viz.level(root.drawn[bar.index]) * root.gain, Theme.vizStyle.knee)))
            }
        }
    }

    // ---- wave: one polygon shared by the fill and the crest outline.
    Shape {
        anchors.fill: parent
        visible: root.shape === "wave"
        preferredRendererType: Shape.GeometryRenderer
        ShapePath {
            strokeWidth: -1
            fillGradient: LinearGradient {
                x1: 0; y1: 0; x2: 0; y2: root.height
                GradientStop { position: 0; color: root.crest }
                GradientStop { position: 1; color: root.base }
            }
            PathPolyline { id: wavePath; objectName: "vizWavePath" }
        }
    }

    // ---- thin bars (S9/N1): one path rebuilt per drawn band set; round caps, flat color.
    Shape {
        anchors.fill: parent
        visible: root.thinBars
        preferredRendererType: Shape.GeometryRenderer
        ShapePath {
            strokeWidth: root.barWidth
            strokeColor: root.crest
            fillColor: Theme.library.clearPaint
            capStyle: ShapePath.RoundCap
            PathMultiline { id: barsPath }
        }
    }

    // ---- wave crest outline (S9/N1)
    Shape {
        anchors.fill: parent
        visible: root.outline && root.shape === "wave"
        preferredRendererType: Shape.GeometryRenderer
        ShapePath {
            strokeWidth: root.outlineWidth
            strokeColor: Qt.rgba(root.color.r, root.color.g, root.color.b, 1)
            fillColor: Theme.library.clearPaint
            joinStyle: ShapePath.RoundJoin
            capStyle: ShapePath.RoundCap
            PathPolyline { id: crestPath }
        }
    }
}
