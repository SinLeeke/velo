import QtQuick
import "../../theme"

// Un solo fundido de una grilla/lista cuando llegan sus PRIMEROS datos y la página ya estaba a
// la vista (no por tarjeta: con delegates reutilizados parpadearía). Opacidad en el hilo de
// render, `fast`, ease-out, una vez por página. Si los datos ya estaban al abrir, no anima.
// Interrumpible: `settle()` deja el estado final; con Modo rendimiento / Reducir movimiento
// (`Theme.motion.fast` = 0) o ventana minimizada no hace nada.
OpacityAnimator {
    id: fade
    property int count: 0
    property bool allowed: true
    property bool armed: false
    Component.onCompleted: armed = count === 0
    onCountChanged: {
        if (!armed || count <= 0) return
        armed = false
        if (!allowed || Theme.motion.fast <= 0 || !target) return
        target.opacity = 0
        restart()
    }
    onAllowedChanged: if (!allowed) settle()
    function settle() {
        if (running) stop()
        if (target) target.opacity = 1
    }
    from: 0
    to: 1
    duration: Math.max(1, Theme.motion.fast)
    easing.type: Easing.Bezier
    easing.bezierCurve: Theme.motion.easeOut
}
