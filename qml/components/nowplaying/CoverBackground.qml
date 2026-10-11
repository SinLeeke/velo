pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Shapes
import "../../theme"

// The cover is the room: the pre-blurred art (Rust blurs once per track, ~96 px) scaled to
// the whole window with bilinear filtering, plus the palette veil. No shader: the blur is
// already baked into the texture.
//
// Two layers (A/B), each an image with its own veil, so a song change crossfades texture AND
// palette together (S1 A2, R2): the new source loads into the hidden back layer; once ready it
// takes the current palette, goes on top and fades 0 → 1 with an OpacityAnimator (render
// thread, Theme.motion.coverFade, easeInOut) over the old one, which keeps its own veil and
// stays opaque underneath until the fade ends. Nothing runs between song changes.
//  - An empty or failed source never replaces a ready texture. The host clears retainEmpty when
//    the session ends; blurOn false always presents the opaque palette, without a texture.
//  - Palette changes are applied one event-loop turn later: if an image comes with them, the
//    palette rides in with that image; alone (light/dark, a late palette) the veil crossfades.
//  - Interruption: a song that arrives mid-fade waits for the fade in flight to end
//    (≤ coverFade), then only the latest one fades in. The incoming layer is always the hidden one.
//  - `blurOn` false («Fondo difuminado» off): no image at all, only the opaque palette veil.
//  - `drift` («Deriva lenta»): a very slow scale/position wander of the layers (Animators,
//    render thread), only while playing, on screen and with motion on.
// coverFade is 0 in Modo rendimiento: the swap is instant.
Item {
    id: root

    property url source
    // Ajustes › Velocidad / Intensidad del fondo (0..100). Speed scales every hypnotic period
    // (50 = the Theme periods, 0 ≈ 2.5× slower, 100 ≈ 2.4× faster); intensity sets how sharp the
    // tiny cover is, how much the swirl and the colour blobs blend in.
    property int speed: 50
    // Beat (0..1, the bass of the visualizer's bands, already smoothed in Rust): in the hypnotic
    // mode the room and the lights swell with it and the lights glow brighter. Transforms and
    // opacity only; 0 with the visualizer off.
    property real pulse: 0
    readonly property real beat: {
        if (!hypnotic || !driftOn)
            return 0
        const lifted = Math.max(0, Math.min(1, (pulse - Theme.nowPlaying.beatFloor) / (1 - Theme.nowPlaying.beatFloor)))
        return Math.pow(lifted, Theme.nowPlaying.beatCurve) * (0.5 + 0.5 * strength)
    }
    property int intensity: 60
    readonly property real pace: 0.4 * Math.pow(6, Math.max(0, Math.min(100, speed)) / 100)
    readonly property real strength: Math.max(0, Math.min(100, intensity)) / 100
    function period(ms) { return Math.max(1000, Math.round(ms / pace)) }
    property bool blurOn: true
    property bool playing: false
    property bool retainEmpty: true
    property color veil: Theme.cover.veil
    property real veilOpacity: Theme.cover.veilOpacity
    property real veilMovingOpacity: Theme.cover.veilMovingOpacity
    // 0 = still (static veil + local scrim), 1 = moving (higher veil, T1). Fades with coverFade.
    property real movingMix: driftOn ? 1 : 0
    Behavior on movingMix { NumberAnimation { duration: Theme.motion.coverFade; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut } }

    // «Hipnótico» (user 2026-10-10: "que los colores se muevan y se mezclen"): the moving room
    // plus a counter-turning swirl layer and the palette's colour blobs. Always the smooth Rust
    // blur (a tiny sharp cover stretched looked like a 360p zoom).
    readonly property bool hypnotic: driftAllowed
    // What the layers should show: nothing without the blurred background.
    readonly property string wanted: blurOn ? String(source) : ""

    property Layer front: layerA      // shown (or fading in, on top)
    property Layer under: null        // outgoing layer, opaque under `front` while it fades in
    property Layer incoming: null     // hidden back layer loading the next source

    function backLayer() { return front === layerA ? layerB : layerA }

    function load() {
        if (fade.running)
            return                       // fadeDone() picks up the latest source
        const want = root.wanted
        if (want === "" && root.blurOn && retainEmpty && front.status === Image.Ready) {
            incoming = null
            return
        }
        if (!incoming && String(front.source) === want)
            return
        incoming = backLayer()
        incoming.source = want
        Qt.callLater(tryFlip)
    }
    function tryFlip() {
        if (!incoming || (String(incoming.source) !== "" && incoming.status !== Image.Ready))
            return
        const old = front
        front = incoming
        incoming = null
        // The palette published with this image (Rust sends both together).
        front.veilColor = root.veil
        front.veilAlpha = root.veilOpacity
        front.veilMovingAlpha = root.veilMovingOpacity
        if (Theme.motion.coverFade > 0 && root.effectsOn && old !== front) {
            under = old
            front.opacity = 0
            fade.target = front
            fade.duration = Theme.motion.coverFade
            fade.start()
        } else {
            front.opacity = 1
            under = null
        }
    }
    function fadeDone() {
        under = null
        load()
        syncVeil()
    }

    // A palette change on its own (no new image in the same turn).
    onVeilChanged: Qt.callLater(syncVeil)
    onVeilOpacityChanged: Qt.callLater(syncVeil)
    onVeilMovingOpacityChanged: Qt.callLater(syncVeil)
    function syncVeil() {
        if (incoming)
            return                       // the incoming image takes the palette when it flips
        if (Qt.colorEqual(front.veilColor, root.veil) && front.veilAlpha === root.veilOpacity
                && front.veilMovingAlpha === root.veilMovingOpacity)
            return
        if (fade.running)
            return                       // every visible layer keeps its palette until the fade ends
        if (Theme.motion.coverFade <= 0 || !root.effectsOn) {
            front.veilColor = root.veil
            front.veilAlpha = root.veilOpacity
        front.veilMovingAlpha = root.veilMovingOpacity
            return
        }
        incoming = backLayer()           // same image, new veil: crossfade the two layers
        incoming.source = front.source
        Qt.callLater(tryFlip)
    }

    // The first texture is set once the item is complete (no fade from nothing, one decode).
    property bool built: false
    onWantedChanged: {
        if (built)
            load()
    }
    onRetainEmptyChanged: if (built) load()
    Component.onCompleted: {
        layerA.source = root.wanted
        layerA.veilColor = root.veil
        layerA.veilAlpha = root.veilOpacity
        layerA.veilMovingAlpha = root.veilMovingOpacity
        front = layerA
        built = true
    }
    // S1-NP: minimized or hidden, an Animator gets no frames and would stay "running": the fade
    // jumps to its end, and a song change while unseen swaps at once.
    readonly property bool windowShown: Window.window !== null && Window.visibility !== Window.Minimized
                                        && Window.visibility !== Window.Hidden

    OpacityAnimator {
        id: fade
        objectName: "backgroundCrossfade"
        from: 0
        to: 1
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeInOut
        onFinished: root.fadeDone()
    }

    // ------------------------------------------------------------ drift («Deriva lenta»)
    readonly property bool effectsOn: visible && playing && windowShown && (!Theme.pauseEffectsInBackground || Window.active)
    onEffectsOnChanged: if (!effectsOn && fade.running) fade.complete()
    Connections {
        target: Theme.motion
        function onCoverFadeChanged() { if (Theme.motion.coverFade === 0 && fade.running) fade.complete() }
    }
    readonly property bool driftAllowed: blurOn && Theme.driftActive && Theme.motion.scale > 0
    readonly property bool driftOn: driftAllowed && playing && effectsOn && front.status === Image.Ready
    // Turned off: the room goes back to rest at once (stopped, it would stay where it was).
    onDriftAllowedChanged: if (!driftAllowed) { room.scale = 1; room.x = 0; room.y = 0; room.rotation = 0 }

    // Paused by `driftOn` (Animators keep the value they reached; restarting goes on from there).
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        ScaleAnimator { target: room; to: Theme.nowPlaying.driftScaleMax; duration: root.period(Theme.nowPlaying.driftScaleMs) / 2; easing.type: Easing.InOutSine }
        ScaleAnimator { target: room; to: Theme.nowPlaying.driftScaleMin; duration: root.period(Theme.nowPlaying.driftScaleMs) / 2; easing.type: Easing.InOutSine }
    }
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        XAnimator { target: room; to: root.width * Theme.nowPlaying.driftShift; duration: root.period(Theme.nowPlaying.driftXMs) / 2; easing.type: Easing.InOutSine }
        XAnimator { target: room; to: -root.width * Theme.nowPlaying.driftShift; duration: root.period(Theme.nowPlaying.driftXMs) / 2; easing.type: Easing.InOutSine }
    }
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        YAnimator { target: room; to: -root.height * Theme.nowPlaying.driftShift; duration: root.period(Theme.nowPlaying.driftYMs) / 2; easing.type: Easing.InOutSine }
        YAnimator { target: room; to: root.height * Theme.nowPlaying.driftShift; duration: root.period(Theme.nowPlaying.driftYMs) / 2; easing.type: Easing.InOutSine }
    }

    // «Hipnótico» (user 2026-10-10): the room also turns slowly on itself, one turn per
    // driftTurnMs. One endless Animator from its current angle to +360 (the same picture): no
    // reset step between turns (that snap was a visible hitch), and a resume goes on from
    // wherever the pause left it.
    RotationAnimator {
        id: turn
        target: room
        loops: Animation.Infinite
        duration: root.period(Theme.nowPlaying.driftTurnMs)
        easing.type: Easing.Linear
    }
    function syncTurn() {
        if (driftOn && !turn.running) {
            turn.from = room.rotation % 360
            turn.to = turn.from + 360
            turn.start()
        } else if (!driftOn && turn.running) {
            turn.stop()
        }
    }
    onDriftOnChanged: syncTurn()
    onPaceChanged: if (turn.running) { turn.stop(); syncTurn() }

    // One layer: the blurred image and the veil it was shown with.
    component Layer: Item {
        id: layer
        property alias source: image.source
        readonly property alias status: image.status
        property color veilColor: Theme.cover.veil
        property real veilAlpha: 1
        property real veilMovingAlpha: 1
        width: canvas.width
        height: canvas.height
        z: root.front === layer ? 1 : 0
        visible: root.front === layer || root.under === layer

        Image {
            id: image
            anchors.fill: parent
            visible: status === Image.Ready
            fillMode: Image.PreserveAspectCrop
            smooth: true
            asynchronous: true
            sourceSize: Qt.size(Theme.cover.blurSourceSize, Theme.cover.blurSourceSize)
            onStatusChanged: Qt.callLater(root.tryFlip)
        }
        // Hypnotic swirl: the same texture (shared in the image cache, no second decode),
        // mirrored and turning the other way at another pace, half see-through: the colours cross
        // and blend. Only in the hypnotic mode; a turn is an Animator (render thread).
        Image {
            id: swirl
            anchors.fill: parent
            visible: root.hypnotic && image.status === Image.Ready
            source: visible ? image.source : ""
            sourceSize: image.sourceSize
            fillMode: Image.PreserveAspectCrop
            mirror: true
            smooth: true
            asynchronous: true
            opacity: Theme.nowPlaying.hypnoticMix * (0.3 + 1.2 * root.strength)
            RotationAnimator on rotation {
                running: swirl.visible && root.driftOn
                loops: Animation.Infinite
                from: 360; to: 0
                duration: root.period(Theme.nowPlaying.hypnoticSwirlMs)
            }
        }
        Rectangle {
            anchors.fill: parent
            color: layer.veilColor
            // Without an image (blur off, no art) the veil is the whole room.
            // T1: while the art moves the veil rises (opacity only) so the text keeps 4.5:1.
            opacity: image.status === Image.Ready ? layer.veilAlpha + (layer.veilMovingAlpha - layer.veilAlpha) * root.movingMix : 1
        }
    }

    // v0.9.2: the layers are a square as wide as the window's diagonal plus the drift's shift, so
    // no turn, shift or scale (≥ 1) ever uncovers an edge (an Animator can be stopped or reset at
    // scale 1 while x/y keep their offset, which left bare stripes). Covers are square, so the
    // crop is natural. Clipped to the view.
    clip: true
    readonly property real canvasSide: Math.ceil(Math.hypot(root.width, root.height) * (1 + 2 * Theme.nowPlaying.driftShift))
    Item {
        id: room
        objectName: "driftRoom"
        width: root.width
        height: root.height
        Item {
            id: canvas
            objectName: "driftCanvas"
            scale: 1 + Theme.nowPlaying.beatRoomScale * root.beat
            x: (root.width - root.canvasSide) / 2
            y: (root.height - root.canvasSide) / 2
            width: root.canvasSide
            height: root.canvasSide
            Layer { id: layerA }
            Layer { id: layerB; opacity: 0 }
        }
    }

    // ------------------------------------------------------------ colour blobs («hipnótico»)
    // More colours, all from the cover: big soft lights in the palette's dominant and vivid
    // colours and their hue neighbours, floating and breathing over the turning art (static
    // radial gradients; only transforms move, Animators on the render thread).
    // Only the cover's own colours (user 2026-10-10: shifted hues and forced saturation made pink
    // and green out of a grey-and-gold cover): its vivid and its dominant colour, at their real
    // hue and (slightly lifted) saturation, each in a few lightness steps placed in the theme's
    // band, deep in dark and light in light, so switching the theme changes them clearly.
    function tone(c, step) {
        const band = Theme.dark ? Theme.nowPlaying.blobLightDark : Theme.nowPlaying.blobLightLight
        const at = band[0] + (band[1] - band[0]) * step
        return Qt.hsla(c.hslHue < 0 ? 0 : c.hslHue,
                       Math.min(1, c.hslSaturation * Theme.nowPlaying.blobSaturationLift), at, 1)
    }
    // T10: the six lights take the cover's five dominant colours (heaviest first; the heaviest twice,
    // in another lightness). Only the lightness changes, never the hue. Without the five colours
    // (old backend) they fall back to the vivid and dominant pair.
    readonly property var blobColors: {
        const list = Theme.cover.colors
        const steps = Theme.nowPlaying.blobLightStep
        const pick = Theme.nowPlaying.blobColorIndex
        if (!list || list.length < 5)
            return [tone(Theme.cover.glow, 0.5), tone(Theme.cover.base, 0.5), tone(Theme.cover.glow, 1),
                    tone(Theme.cover.base, 0), tone(Theme.cover.glow, 0), tone(Theme.cover.base, 1)]
        return pick.map((c, i) => tone(Qt.color(list[c]), steps[i]))
    }
    Item {
        id: blobs
        objectName: "hypnoticBlobs"
        anchors.fill: parent
        visible: root.hypnotic && root.front.status === Image.Ready && opacity > 0
        // T1: while the art moves the lights are capped (Rust, per cover) so the text keeps 4.5:1.
        opacity: Math.min(1 - (1 - Theme.cover.blobMovingOpacity) * root.movingMix,
                          Theme.nowPlaying.blobAlpha * root.strength + Theme.nowPlaying.beatGlow * root.beat)
        scale: 1 + Theme.nowPlaying.beatBlobScale * root.beat
        Repeater {
            model: root.blobColors.length
            delegate: Item {
                id: blob
                required property int index
                readonly property real side: Math.max(root.width, root.height) * Theme.nowPlaying.blobSize
                // Each light wanders between two corners of its own, on its own (prime) periods.
                // Paths cross the middle so the lights meet and blend; prime periods never sync.
                readonly property var from: [[-0.2, -0.2], [0.6, -0.25], [0.65, 0.6], [-0.25, 0.55], [0.25, -0.35], [0.3, 0.75]][index]
                readonly property var to:   [[0.5, 0.45], [-0.1, 0.5], [0.0, -0.1], [0.6, -0.1], [0.4, 0.6], [-0.15, 0.1]][index]
                readonly property int ms: [17000, 19000, 23000, 29000, 31000, 37000][index]
                width: side; height: side
                x: from[0] * root.width; y: from[1] * root.height
                Shape {
                    anchors.fill: parent
                    preferredRendererType: Shape.GeometryRenderer
                    ShapePath {
                        strokeWidth: -1
                        fillGradient: RadialGradient {
                            centerX: blob.side / 2; centerY: blob.side / 2
                            centerRadius: blob.side / 2
                            focalX: centerX; focalY: centerY
                            GradientStop { position: 0; color: root.blobColors[blob.index] }
                            // A solid core before the falloff: a shape of colour, not a haze.
                            GradientStop { position: Theme.nowPlaying.blobCore; color: root.blobColors[blob.index] }
                            GradientStop { position: 1; color: Qt.rgba(root.blobColors[blob.index].r, root.blobColors[blob.index].g, root.blobColors[blob.index].b, 0) }
                        }
                        PathRectangle { width: blob.side; height: blob.side }
                    }
                }
                SequentialAnimation {
                    running: root.driftOn && blobs.visible
                    loops: Animation.Infinite
                    ParallelAnimation {
                        XAnimator { target: blob; to: blob.to[0] * root.width; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                        YAnimator { target: blob; to: blob.to[1] * root.height; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                        ScaleAnimator { target: blob; to: Theme.nowPlaying.blobBreath; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                    }
                    ParallelAnimation {
                        XAnimator { target: blob; to: blob.from[0] * root.width; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                        YAnimator { target: blob; to: blob.from[1] * root.height; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                        ScaleAnimator { target: blob; to: 1; duration: root.period(blob.ms); easing.type: Easing.InOutSine }
                    }
                }
            }
        }
    }
}
