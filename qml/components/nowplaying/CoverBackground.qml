pragma ComponentBehavior: Bound
import QtQuick
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
    property bool blurOn: true
    property bool playing: false
    property bool retainEmpty: true
    property color veil: Theme.cover.veil
    property real veilOpacity: Theme.cover.veilOpacity

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
    function syncVeil() {
        if (incoming)
            return                       // the incoming image takes the palette when it flips
        if (Qt.colorEqual(front.veilColor, root.veil) && front.veilAlpha === root.veilOpacity)
            return
        if (fade.running)
            return                       // every visible layer keeps its palette until the fade ends
        if (Theme.motion.coverFade <= 0 || !root.effectsOn) {
            front.veilColor = root.veil
            front.veilAlpha = root.veilOpacity
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
    onDriftAllowedChanged: if (!driftAllowed) { room.scale = 1; room.x = 0; room.y = 0 }

    // Paused by `driftOn` (Animators keep the value they reached; restarting goes on from there).
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        ScaleAnimator { target: room; to: Theme.nowPlaying.driftScaleMax; duration: Theme.nowPlaying.driftScaleMs / 2; easing.type: Easing.InOutSine }
        ScaleAnimator { target: room; to: Theme.nowPlaying.driftScaleMin; duration: Theme.nowPlaying.driftScaleMs / 2; easing.type: Easing.InOutSine }
    }
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        XAnimator { target: room; to: root.width * Theme.nowPlaying.driftShift; duration: Theme.nowPlaying.driftXMs / 2; easing.type: Easing.InOutSine }
        XAnimator { target: room; to: -root.width * Theme.nowPlaying.driftShift; duration: Theme.nowPlaying.driftXMs / 2; easing.type: Easing.InOutSine }
    }
    SequentialAnimation {
        running: root.driftOn
        loops: Animation.Infinite
        YAnimator { target: room; to: -root.height * Theme.nowPlaying.driftShift; duration: Theme.nowPlaying.driftYMs / 2; easing.type: Easing.InOutSine }
        YAnimator { target: room; to: root.height * Theme.nowPlaying.driftShift; duration: Theme.nowPlaying.driftYMs / 2; easing.type: Easing.InOutSine }
    }

    // One layer: the blurred image and the veil it was shown with.
    component Layer: Item {
        id: layer
        property alias source: image.source
        readonly property alias status: image.status
        property color veilColor: Theme.cover.veil
        property real veilAlpha: 1
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
        Rectangle {
            anchors.fill: parent
            color: layer.veilColor
            // Without an image (blur off, no art) the veil is the whole room.
            opacity: image.status === Image.Ready ? layer.veilAlpha : 1
        }
    }

    // v0.9.2: the layers overscan the room by driftOverscan on every side, so the drift's shift
    // never uncovers an edge whatever the scale is at that moment (an Animator can be stopped or
    // reset at scale 1 while x/y keep their offset, which left bare stripes). Clipped to the view.
    clip: true
    readonly property real overscanX: Math.ceil(root.width * Theme.nowPlaying.driftOverscan)
    readonly property real overscanY: Math.ceil(root.height * Theme.nowPlaying.driftOverscan)
    Item {
        id: room
        objectName: "driftRoom"
        width: root.width
        height: root.height
        Item {
            id: canvas
            objectName: "driftCanvas"
            x: -root.overscanX
            y: -root.overscanY
            width: root.width + 2 * root.overscanX
            height: root.height + 2 * root.overscanY
            Layer { id: layerA }
            Layer { id: layerB; opacity: 0 }
        }
    }
}
