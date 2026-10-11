import QtQuick
import QtQuick.Controls.impl
import QtQuick.Effects
import Velo.Backend as Backend
import "../theme"
import "library"
import "."

// Alerta reutilizable: entrada y salida por transformación/opacidad, sin bucles.
//   show(text, action, kind): `action` (opcional) es el texto del botón (actionRequested al
//   pulsarlo); `kind` es "error" (rojo, por defecto) o "notice" (aviso que no es un fallo).
// El plazo (Theme.toast.timeout; más largo con una acción) se detiene mientras el puntero o el
// foco del teclado están sobre el aviso, y con la ventana minimizada: nunca vence sin que se
// haya podido leer. Lectores de pantalla: el contenedor entero es la alerta.
Item {
    id: root
    property string message: ""
    property string actionText: ""
    property string kind: "error"       // "error" | "notice"
    signal actionRequested()
    property bool shown: false
    property bool armed: false
    readonly property bool isError: kind !== "notice"
    readonly property var window: root.Window.window
    readonly property bool windowVisible: visible && (!window || (window.visibility !== Window.Minimized && window.visibility !== Window.Hidden))
    readonly property bool effectsAllowed: windowVisible && Theme.interactionAllowed && Theme.motion.scale > 0
        && (!window || window.active || !Theme.pauseEffectsInBackground)
    // Reading or reaching the action holds the countdown (WCAG 2.2.1).
    readonly property bool held: hover.hovered || actionButton.activeFocus || closeButton.activeFocus
    function show(text, action, severity) {
        const wasLeaving = dismissing
        const alreadyUp = shown && !wasLeaving
        if (!alreadyUp) incoming.stop()
        outgoing.stop(); deadline.stop(); armed = false
        if (action !== undefined) actionText = action
        dismissing = false
        kind = severity === "notice" ? "notice" : "error"
        message = text; shown = true
        // A second notice while one is on screen only swaps the text: no second entrance.
        if (!alreadyUp) {
            card.opacity = 0; card.y = -card.height - root.y
            if (effectsAllowed) incoming.start()
            else { card.opacity = 1; card.y = 0 }
        }
        armed = true            // re-arming restarts the countdown
    }
    function dismiss() {
        armed = false; incoming.stop()
        dismissing = true
        if (effectsAllowed) outgoing.restart()
        else { outgoing.stop(); shown = false; message = "" }
    }
    property bool dismissing: false
    onEffectsAllowedChanged: if (!effectsAllowed) {
        incoming.stop(); outgoing.stop(); card.y = 0; card.opacity = 1
        if (dismissing) { shown = false; message = "" }
    }
    visible: shown
    width: Math.min(parent.width - 2 * Theme.space.gutter, Theme.toast.width)
    height: card.height
    x: (parent.width - width) / 2
    y: Theme.size.titleBar + Theme.space.md
    z: 20
    Accessible.role: Accessible.AlertMessage
    Accessible.name: message
    HoverHandler { id: hover }
    // v0.9.2 (user: "este cartel está muy feo y no se da a entender que hay un botón de volver a
    // intentar"): floating card (shadow, no red border), a short colour rail for the kind, the
    // message on the left and the action as a compact primary button on the right.
    Item {
        id: card
        width: root.width; height: Math.max(body.height, actionButton.visible ? actionButton.height : 0) + 2 * Theme.space.md
        RectangularShadow {
            anchors.fill: face; radius: face.radius
            offset.y: Theme.elevation.popover.offsetY
            blur: Theme.elevation.popover.blur
            spread: Theme.elevation.popover.spread
            color: Theme.elevation.popoverShadow
        }
        Rectangle {
            id: face
            anchors.fill: parent; radius: Theme.radius.lg; color: Theme.surface2
            clip: true
            Rectangle {   // kind rail
                width: Theme.space.xs; height: parent.height
                color: root.isError ? Theme.danger : Theme.accent
            }
        }
        IconImage {
            x: Theme.space.lg; anchors.verticalCenter: parent.verticalCenter
            source: Qt.resolvedUrl("../icons/info.svg")
            sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
            color: root.isError ? Theme.danger : Theme.accent
        }
        Column {
            id: body
            x: Theme.space.lg * 2 + Theme.icon.md; anchors.verticalCenter: parent.verticalCenter
            width: parent.width - x - closeButton.width - Theme.space.sm
                   - (actionButton.visible ? actionButton.width + Theme.space.sm : 0)
            LibraryLabel {
                width: parent.width; text: root.message
                wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
                Accessible.ignored: true      // read through the container's alert
            }
        }
        LibraryButton {
            id: actionButton
            objectName: "toastAction"
            visible: root.actionText !== ""; text: root.actionText
            primary: true
            anchors.right: closeButton.left; anchors.rightMargin: Theme.space.xs
            anchors.verticalCenter: parent.verticalCenter
            onClicked: { root.actionRequested(); root.dismiss() }
        }
        IconButton {
            id: closeButton
            anchors.right: parent.right; anchors.rightMargin: Theme.space.xs
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Cerrar aviso"); iconSource: Qt.resolvedUrl("../icons/close.svg")
            onClicked: root.dismiss()
        }
    }
    ParallelAnimation {
        id: incoming
        YAnimator { target: card; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        OpacityAnimator { target: card; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    ParallelAnimation {
        id: outgoing
        YAnimator { target: card; to: -card.height - root.y; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeIn }
        OpacityAnimator { target: card; to: 0; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeIn }
        onFinished: { root.shown = false; root.message = "" }
    }
    // Runs only while shown, visible and not held; holding restarts the full time on release.
    Timer {
        id: deadline
        interval: root.actionText !== "" ? Theme.toast.actionTimeout : Theme.toast.timeout
        repeat: false
        running: root.armed && root.shown && root.windowVisible && Backend.Player.isPlaying && !root.held
        onTriggered: root.dismiss()
    }
}
