import QtQuick
import "../../theme"

// Primer uso: una entrada, sin Timer ni bucle. Opacidad en el hilo de render;
// Translate.y usa NumberAnimation porque Translate no es un Item para YAnimator.
SequentialAnimation {
    id: reveal
    property Item item
    property Translate shift
    property int order: 0
    property bool effectsAllowed: true
    onEffectsAllowedChanged: if (!effectsAllowed) settle()
    function begin() {
        if (!effectsAllowed || Theme.motion.scale === 0) return
        shift.y = Theme.entrance.cardShift
        start()
    }
    function settle() {
        if (running) stop()
        if (item) item.opacity = 1
        if (shift) shift.y = 0
    }
    OpacityAnimator { target: reveal.item; from: 0; to: 0; duration: Math.max(1, reveal.order * Theme.entrance.stagger) }
    ParallelAnimation {
        OpacityAnimator { target: reveal.item; from: 0; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        NumberAnimation { target: reveal.shift; property: "y"; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
}
