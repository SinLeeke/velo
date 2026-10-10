import QtQuick
import QtQuick.Shapes
import QtQuick.Effects
import QtQuick.Templates as T
import Velo.Backend
import "../../theme"
import "../motion/morph.js" as Morph

// Cover art in two modes:
//  - square: rounded rect (Theme.radius.xl) lifted by a RectangularShadow (Theme.elevation.cover)
//  - circular: a perfect circle (T4, the user's request: it used to be a scalloped "cookie"),
//    used when Visualizer.style == "circular"
// The art is painted by a Shape whose ShapePath.fillItem samples the Image texture (Qt 6.8+),
// so the clipping costs no shader or layer: the circle is the same PathRectangle as the square
// with its radius at half the side (PathRectangle clamps it), so both modes share one path.
// The whole item is the slot; in circular mode `ringSlot` (the full slot) is left free for the
// circular visualizer (VisualizerRing, loaded by NowPlaying) and the circle sits centered inside
// it; the ring's inner radius is artSize / 2, the circle's own radius.
// v0.9.2: `frame` (square mode with a visualizer): the art shrinks to squareArtRatio of the slot
// and `ringSlot` stays free for the square reactive frame around it (VisualizerRing, square).
// T4: the art itself is a button (`artButton`, round in circular mode): a click, Enter or Space
// emits artClicked() (v0.9.2: NowPlaying toggles circle ↔ square). Clicks on the slot around the
// art fall through to whatever is behind.
//
// S1-NP:
//  - Cover change (a new song, the DJ's portrait ↔ the album art): two faces take turns. The new
//    image decodes in the hidden face; once ready it goes on top and fades in (OpacityAnimator,
//    Theme.motion.artFade) while settling artFadeZoom → 1 (ScaleAnimator, skipped when motion is
//    off). The old face stays opaque underneath until the fade ends, then lets its texture go.
//    R2: empty and failed URLs keep the last ready face until another image is ready. The host
//    clears retainEmpty when the session ends; no timer can discard a texture during a slow load.
//  - Card tilt: a HoverHandler over the art (hover only: it takes no clicks and does not block
//    window dragging) aims two spring-smoothed values (-1..1). They drive one projective
//    Matrix4x4 (perspective tilt, transform only), a specular glare (a radial gradient Shape over
//    the art, opacity only) and a shadow Translate. Springs stop once settled: nothing runs while
//    the pointer is still or away. Flat when `tiltEnabled` is false or Theme.motion.scale is 0.
//  - `ringReveal` (0..1, eased by NowPlaying) opens the circular visualizer slot.
Item {
    id: root

    property url source
    property bool circular: false
    property bool frame: false           // square art with room for the reactive frame (v0.9.2)
    property alias ringSlot: ring
    property bool tiltEnabled: true      // NowPlaying turns it off while the cover is flying
    property bool playing: Player.isPlaying
    property bool retainEmpty: true
    property real ringReveal: 1          // S1-NP entrance of the circular visualizer
    // A face shows a decoded image (false while the first image loads, or with no image).
    readonly property bool hasArt: front.status === Image.Ready
    signal artClicked()
    property string artName: ""          // accessible name of the art button (qsTr from the host)
    // T4b: true while the press being delivered only closes a popup (the host knows): a click
    // that starts that way is not the cover's, until its release.
    property bool pressBlocked: false
    // T8: the art is a button only while its click does something visible (the host: a
    // visualizer is drawn). Off, it takes no press, no Tab stop and shows no hand cursor.
    property bool artEnabled: true
    readonly property alias artButton: artButton

    readonly property real artSize: Math.min(width, height) * (circular ? Theme.nowPlaying.cookieRatio
                                                               : frame ? Theme.nowPlaying.squareArtRatio : 1)
    readonly property real artCorner: art.cornerRadius

    // Visualizer ring/blob (circle) or reactive frame (square) goes here, behind the art.
    Item {
        id: ring
        objectName: "vizRingSlot"
        anchors.fill: parent
        visible: (root.circular || root.frame) && opacity > 0
        opacity: root.ringReveal
        scale: Morph.mix(Theme.motion.npRingFrom, 1, root.ringReveal)
        // The square frame does not turn: the art it hugs never does.
        rotation: root.circular ? (1 - root.ringReveal) * Theme.motion.npRingTurn : 0
    }

    // ------------------------------------------------------------ tilt
    // Off while minimized/hidden (and unfocused with "Pausar efectos en segundo plano"): an
    // unseen tilt or crossfade jumps to its end, also when playback pauses.
    readonly property bool effectsOn: playing && visible && root.Window.window !== null
        && root.Window.window.visible && root.Window.visibility !== Window.Minimized
        && root.Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || root.Window.active)
    readonly property bool tiltOn: tiltEnabled && Theme.motion.scale > 0 && effectsOn
    property real aimX: 0                // pointer, -1 (left) .. 1 (right), spring-smoothed
    property real aimY: 0                // pointer, -1 (top) .. 1 (bottom), spring-smoothed
    property real lift: 1                // 1 → Theme.motion.tiltLift while hovered
    readonly property real hoverAmount: Theme.motion.tiltLift > 1
                                        ? Morph.clamp01((lift - 1) / (Theme.motion.tiltLift - 1)) : 0
    // Perspective tilt of the art (the point under the pointer sinks back), computed once per
    // spring step and shared by the drawn art and its button (T8: it was computed twice).
    readonly property matrix4x4 tiltMatrix: aimX === 0 && aimY === 0 ? Qt.matrix4x4()
        : Qt.matrix4x4(Morph.tilt(artSize / 2, artSize / 2, -aimY * Theme.motion.tiltMax,
                                  aimX * Theme.motion.tiltMax, Theme.motion.tiltPerspective))
    // G4 (ronda 2, «que siempre haga algo sea donde sea que ponga el mouse»): the aim is measured
    // from the art's centre in the hover zone's own (untransformed) coordinates and saturates at
    // the art's edge, so any point of the zone (the lifted rim, the ring around the circle, the
    // slop) tilts toward the side the pointer is on.
    function aim() {
        const on = tiltOn && hover.hovered
        const half = Math.max(1, artSize / 2)
        const cx = hoverZone.width / 2, cy = hoverZone.height / 2
        aimX = on ? Math.max(-1, Math.min(1, (hover.point.position.x - cx) / half)) : 0
        aimY = on ? Math.max(-1, Math.min(1, (hover.point.position.y - cy) / half)) : 0
        lift = on ? Theme.motion.tiltLift : 1
    }
    onTiltOnChanged: aim()
    Behavior on aimX {
        enabled: root.tiltOn
        SpringAnimation { id: springX; spring: Theme.motion.tiltSpring; damping: Theme.motion.tiltDamping; epsilon: 0.002 }
    }
    Behavior on aimY {
        enabled: root.tiltOn
        SpringAnimation { id: springY; spring: Theme.motion.tiltSpring; damping: Theme.motion.tiltDamping; epsilon: 0.002 }
    }
    Behavior on lift {
        enabled: root.tiltOn
        SpringAnimation { id: springLift; spring: Theme.motion.tiltSpring; damping: Theme.motion.tiltDamping; epsilon: 0.0005 }
    }

    // Untransformed zone around the art: hover positions stay stable while the art tilts. G4: it
    // covers the whole slot (in circular mode the visualizer ring too) and the art at its lifted
    // size plus Theme.motion.tiltSlop, so the lift never pulls the edge out from under the pointer
    // (which used to make it flicker or ignore the rim). The handler is always on (passive, it
    // costs nothing at rest): when the tilt comes back on (after the cover's flight, on restore)
    // `hovered` is already right, even if the pointer has not moved.
    Item {
        id: hoverZone
        anchors.centerIn: parent
        width: Math.max(root.width, root.artSize * Math.max(1, Theme.motion.tiltLift)) + 2 * Theme.motion.tiltSlop
        height: Math.max(root.height, root.artSize * Math.max(1, Theme.motion.tiltLift)) + 2 * Theme.motion.tiltSlop
        HoverHandler {
            id: hover
            onPointChanged: root.aim()
            onHoveredChanged: root.aim()
        }
    }

    RectangularShadow {
        readonly property var e: Theme.elevation.cover
        anchors.fill: art
        radius: root.circular ? width / 2 : Theme.radius.xl
        // Light from the viewer: the shadow slides away from the side that sinks.
        offset.y: e.offsetY
        transform: Translate {
            x: -root.aimX * Theme.motion.tiltShadowShift
            y: -root.aimY * Theme.motion.tiltShadowShift
        }
        scale: root.lift
        blur: e.blur
        spread: e.spread
        color: Qt.rgba(0, 0, 0, e.alpha)
    }

    // ------------------------------------------------------------ faces (A/B crossfade)
    property Item front: faceA           // shown, or fading in on top
    property Item incoming: null         // hidden face decoding the next image
    function backFace() { return front === faceA ? faceB : faceA }
    function load() {
        if (fade.running)
            return                       // fadeDone() picks up the latest source
        const want = String(root.source)
        if (want === "" && retainEmpty && front.status === Image.Ready) {
            incoming = null
            return
        }
        if (!incoming && String(front.source) === want)
            return
        incoming = backFace()
        incoming.source = want
        Qt.callLater(tryFlip)
    }
    function tryFlip() {
        if (!incoming || (String(incoming.source) !== "" && incoming.status !== Image.Ready))
            return
        const old = front
        front = incoming
        incoming = null
        front.z = 2
        old.z = 1
        if (effectsOn && Theme.motion.artFade > 0 && old.status === Image.Ready && front.status === Image.Ready) {
            front.opacity = 0
            fade.target = front
            fade.duration = Theme.motion.artFade
            fade.start()
            if (Theme.motion.scale > 0) {
                front.scale = Theme.motion.artFadeZoom
                settle.target = front
                settle.duration = Theme.motion.artFade
                settle.start()
            }
        } else {
            front.opacity = 1
            front.scale = 1
            old.source = ""
        }
    }
    function fadeDone() {
        const old = backFace()
        if (old !== incoming)
            old.source = ""              // its texture is no longer shown
        load()
    }
    onSourceChanged: load()
    onRetainEmptyChanged: load()
    function finishEffects() {
        if (effectsOn && Theme.motion.scale > 0)
            return
        // Animators retain their render-thread value until stopped/completed.
        if (settle.running) settle.complete()
        if (fade.running) fade.complete()
        springX.stop()
        springY.stop()
        springLift.stop()
        aim()
    }
    onEffectsOnChanged: finishEffects()
    Connections {
        target: Theme.motion
        function onScaleChanged() { root.finishEffects() }
    }
    Component.onCompleted: { faceA.source = root.source; front = faceA }

    OpacityAnimator {
        id: fade
        objectName: "artCrossfade"
        from: 0
        to: 1
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeInOut
        onFinished: root.fadeDone()
    }
    ScaleAnimator {
        id: settle
        to: 1
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeOut
    }

    // One face: the image (texture source only, never drawn directly) and the Shape that
    // paints it clipped to the rounded square or the circle.
    component Face: Item {
        id: face
        property alias source: image.source
        readonly property alias status: image.status
        anchors.fill: parent
        visible: status === Image.Ready

        Image {
            id: image
            anchors.fill: parent
            visible: false
            sourceSize: Qt.size(Theme.size.coverMax * Screen.devicePixelRatio, Theme.size.coverMax * Screen.devicePixelRatio)
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            smooth: true
            mipmap: true
            onStatusChanged: Qt.callLater(root.tryFlip)
        }
        // fillItem maps the texture at its decoded pixel size; scale it onto the art box.
        readonly property matrix4x4 textureFit: PlanarTransform.fromScale(
            width / Math.max(1, image.implicitWidth), height / Math.max(1, image.implicitHeight))

        Shape {
            anchors.fill: parent
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                // The circle keeps the soft rim the cookie had.
                strokeColor: root.circular ? Theme.cover.pillHover : Theme.library.clearPaint
                strokeWidth: root.circular ? Theme.stroke.focus : -1
                fillItem: image
                fillTransform: face.textureFit
                PathRectangle { width: face.width; height: face.height; radius: art.cornerRadius }
            }
        }
    }

    Item {
        id: art
        anchors.centerIn: parent
        width: root.artSize
        height: root.artSize
        scale: root.lift
        // Perspective tilt: the point under the pointer sinks back (transform only, no layer).
        transform: Matrix4x4 { matrix: root.tiltMatrix }

        // Rounded square, or a circle (radius = half the side) in circular mode.
        readonly property real cornerRadius: root.circular ? width / 2 : Theme.radius.xl

        // Placeholder while the first image loads (and with no image at all).
        Shape {
            anchors.fill: parent
            preferredRendererType: Shape.CurveRenderer
            visible: !root.hasArt
            ShapePath {
                strokeWidth: -1
                fillColor: Theme.cover.pill
                PathRectangle { width: art.width; height: art.height; radius: art.cornerRadius }
            }
        }

        Face { id: faceA }
        Face { id: faceB; opacity: 0 }

        // Specular glare uses a fixed gradient: hover changes opacity, never its paint geometry.
        Shape {
            anchors.fill: parent
            preferredRendererType: Shape.CurveRenderer
            visible: opacity > 0
            opacity: root.hoverAmount
            z: 3
            ShapePath {
                strokeWidth: -1
                fillGradient: RadialGradient {
                    centerX: art.width / 2
                    centerY: art.height / 2
                    centerRadius: art.width * Theme.motion.tiltGlareRadius
                    focalX: centerX
                    focalY: centerY
                    GradientStop { position: 0; color: Qt.rgba(1, 1, 1, Theme.motion.tiltGlare) }
                    GradientStop { position: 1; color: Qt.rgba(1, 1, 1, 0) }
                }
                PathRectangle { width: art.width; height: art.height; radius: art.cornerRadius }
            }
        }
    }

    // T4: the art is a button over the tilting art. Round in circular mode (containmentMask),
    // so a click on the ring around the circle is not the cover's. T4b: it carries the art's
    // own lift (scale) and perspective tilt, so its hit area is the art as drawn (the lifted rim
    // too); transforms only, no relayout. Keyboard: a Tab stop with a ring outside the art;
    // Enter or Space presses it. No motion of its own: the visualizer's shape change is the
    // feedback.
    T.AbstractButton {
        id: artButton
        objectName: "npCoverButton"
        anchors.centerIn: parent
        width: root.artSize
        height: root.artSize
        scale: root.lift
        transform: Matrix4x4 { matrix: root.tiltMatrix }
        enabled: root.artEnabled
        focusPolicy: Qt.TabFocus
        // T8: the only pointer hint of the cover's click (the user asked for no visible chrome):
        // the hand over the art itself (containmentMask: the circle, not the ring around it).
        HoverHandler { cursorShape: Qt.PointingHandCursor }
        hoverEnabled: false               // the hover zone above drives the tilt
        text: root.artName
        Accessible.role: Accessible.Button
        Accessible.name: text
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: root.artClicked()
        Keys.onEnterPressed: root.artClicked()
        // Pointer: decided at the press (a press that closes a popup never toggles); Space also
        // presses it, never blocked.
        property bool armed: true
        onPressed: armed = !root.pressBlocked
        onClicked: if (armed) root.artClicked()
        containmentMask: QtObject {
            function contains(point: point): bool {
                if (!root.circular)
                    return point.x >= 0 && point.y >= 0 && point.x <= artButton.width && point.y <= artButton.height
                const r = artButton.width / 2, dx = point.x - r, dy = point.y - r
                return dx * dx + dy * dy <= r * r
            }
        }
        background: Rectangle {
            readonly property real gap: Theme.nowPlayingClicks.focusGap + Theme.stroke.focus
            x: -gap
            y: -gap
            width: artButton.width + 2 * gap
            height: artButton.height + 2 * gap
            radius: root.circular ? width / 2 : Theme.radius.xl + gap
            visible: artButton.visualFocus
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.cover.ink
        }
    }
}
