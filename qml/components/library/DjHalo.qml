import QtQuick
import Velo.Backend as Backend
import "../../theme"

// E5b: "Livi is talking". A soft accent disc behind the DJ glyph that breathes (grows and
// brightens, then settles: scale + opacity on render-thread Animators, Theme.dj.pulse per half).
// It fades in when Livi starts a line and out when the song comes back. The loop runs only while
// it can be seen and motion is on: it stops with the music paused, the window minimized or
// hidden, in the background with "Pausar efectos en segundo plano", and with Modo rendimiento /
// Reducir movimiento (a still halo then marks that Livi is speaking).
Item {
    id: halo
    property color color: Theme.accent
    readonly property bool speaking: Backend.Player.djActive && Backend.Player.djSpeaking
    readonly property var window: halo.Window.window
    readonly property bool breathing: speaking && visible && Backend.Player.isPlaying && Theme.effectsAllowed && Theme.dj.pulse > 0
        && (!window || (window.visibility !== Window.Minimized && window.visibility !== Window.Hidden
            && (window.active || !Theme.pauseEffectsInBackground)))
    opacity: speaking ? 1 : 0
    Behavior on opacity {
        OpacityAnimator { duration: Math.max(1, Theme.motion.slow); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    Rectangle {
        id: disc
        anchors.fill: parent
        radius: width / 2
        color: halo.color
        opacity: Theme.dj.haloRest
    }
    SequentialAnimation {
        id: breath
        running: halo.breathing
        loops: Animation.Infinite
        ParallelAnimation {
            ScaleAnimator { target: disc; from: 1; to: Theme.dj.pulseScale; duration: Theme.dj.pulse; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            OpacityAnimator { target: disc; from: Theme.dj.haloRest; to: Theme.dj.haloPeak; duration: Math.max(1, Theme.dj.pulse); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        }
        ParallelAnimation {
            ScaleAnimator { target: disc; from: Theme.dj.pulseScale; to: 1; duration: Theme.dj.pulse; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            OpacityAnimator { target: disc; from: Theme.dj.haloPeak; to: Theme.dj.haloRest; duration: Math.max(1, Theme.dj.pulse); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        }
        // A stopped loop leaves the disc mid-breath: put it back at rest.
        onRunningChanged: if (!running) Qt.callLater(halo.settle)
    }
    function settle() {
        if (breath.running) return
        disc.scale = 1
        disc.opacity = Theme.dj.haloRest
    }
}
