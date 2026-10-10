import QtQuick
import "../theme"

// G4: a vertical drag handle on the edge of a side panel (library sidebar, queue panel). Place it
// centered on the edge (x = edge - width / 2) at full height. Dragging reports the new width live
// (`resized`, it follows the pointer with no animation) and once more on release (`committed`, the
// moment to store it); a double click asks for the default width (`resetRequested`).
// `growsRight`: true when dragging to the right widens the panel (handle on its right edge).
// Hover shows a thin line (opacity fade, `fast`) and the horizontal resize cursor. No Timer.
// Keyboard (R1): a Tab stop; ←/→ move the edge by Theme.resize.keyStep (in the direction it
// moves on screen), Inicio/Fin jump to the minimum / maximum, Supr/Retroceso restore the
// default. Each key reports `resized` and `committed` at once. Focus by keyboard shows the line.
Item {
    id: handle

    property real value: 0                 // the panel's current width
    property real minimum: 0
    property real maximum: 0
    property bool growsRight: true
    signal resized(real width)
    signal committed(real width)
    signal resetRequested()
    readonly property bool dragging: area.pressed
    // Only Tab gives it focus (a drag does not take it), so its focus is always the keyboard's.
    readonly property bool keyboardFocus: activeFocus

    width: Theme.resize.handle
    activeFocusOnTab: true

    readonly property real minimumValue: minimum
    readonly property real maximumValue: maximum
    readonly property real stepSize: Theme.resize.keyStep
    Accessible.role: Accessible.Slider
    Accessible.name: qsTr("Cambiar el ancho del panel")
    Accessible.description: qsTr("%1 píxeles. Flechas izquierda y derecha para cambiarlo; Inicio y Fin para el mínimo y el máximo.").arg(Math.round(value))
    Accessible.focusable: true

    function setWidth(w) {
        const clamped = Math.round(Math.max(minimum, Math.min(Math.max(minimum, maximum), w)))
        resized(clamped)
        committed(clamped)
    }
    Keys.onPressed: event => {
        const step = growsRight ? Theme.resize.keyStep : -Theme.resize.keyStep
        if (event.key === Qt.Key_Right) setWidth(value + step)
        else if (event.key === Qt.Key_Left) setWidth(value - step)
        else if (event.key === Qt.Key_Home) setWidth(minimum)
        else if (event.key === Qt.Key_End) setWidth(maximum)
        else if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) resetRequested()
        else return
        event.accepted = true
    }

    Rectangle {
        anchors.centerIn: parent
        width: Theme.resize.line
        height: Math.max(0, parent.height - 2 * Theme.resize.lineInset)
        radius: width / 2
        color: handle.dragging || handle.keyboardFocus ? Theme.resize.lineActive : Theme.resize.lineColor
        // Opacity 0 is skipped by the scene graph; no `visible` binding, which an Animator only
        // updates once it ends (the fade-in would never show).
        opacity: area.containsMouse || handle.dragging || handle.keyboardFocus ? 1 : 0
        Behavior on opacity {
            OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.SizeHorCursor
        preventStealing: true
        property real startX: 0
        property real startValue: 0
        function widthAt(mouseX) {
            const dx = handle.mapToItem(null, mouseX, 0).x - startX
            const w = startValue + (handle.growsRight ? dx : -dx)
            return Math.round(Math.max(handle.minimum, Math.min(Math.max(handle.minimum, handle.maximum), w)))
        }
        onPressed: mouse => {
            startX = handle.mapToItem(null, mouse.x, 0).x
            startValue = handle.value
        }
        onPositionChanged: mouse => { if (pressed) handle.resized(widthAt(mouse.x)) }
        onReleased: mouse => handle.committed(widthAt(mouse.x))
        onDoubleClicked: handle.resetRequested()
    }
}
