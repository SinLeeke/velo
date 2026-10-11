import QtQuick
import "../../theme"

// Entrada de una sección de página (mismo gesto que `Section` de Inicio): opacidad 0 -> 1 y subida
// de `entrance.cardShift`, `base`, ease-out, con retraso `min(order, staggerMax) * stagger`.
// Se dispara UNA vez, cuando la sección pasa a visible DESPUÉS de crearse la página (datos que
// llegan tarde); si ya estaba ahí al abrir, no se anima. Sin Timer: el retraso es un Animator.
// Solo opacidad y Translate (sin relayout). Con Modo rendimiento / Reducir movimiento queda
// instantáneo; `settle()` salta al estado final.
SequentialAnimation {
    id: reveal
    property Item item
    property Translate shift
    property int order: 0
    property bool effectsAllowed: true
    property bool done: false
    readonly property Connections watcher: Connections {
        target: reveal.item
        function onVisibleChanged() { if (reveal.item.visible) reveal.begin() }
    }
    onEffectsAllowedChanged: if (!effectsAllowed) settle()
    function begin() {
        if (done) return
        done = true
        if (!effectsAllowed || Theme.motion.base <= 0 || !item || !shift) return
        item.opacity = 0
        shift.y = Theme.entrance.cardShift
        restart()
    }
    function settle() {
        if (running) stop()
        if (item) item.opacity = 1
        if (shift) shift.y = 0
    }
    OpacityAnimator { target: reveal.item; from: 0; to: 0; duration: Math.max(1, Math.min(reveal.order, Theme.entrance.staggerMax) * Theme.entrance.stagger) }
    ParallelAnimation {
        OpacityAnimator { target: reveal.item; from: 0; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        NumberAnimation { target: reveal.shift; property: "y"; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
}
