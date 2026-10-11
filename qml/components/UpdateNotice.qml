import QtQuick
import QtQuick.Effects
import Velo.Backend as Backend
import "../theme"
import "library"

// A persistent companion to Toast: the user decides when an available update goes away.
// v0.9.2 (user: "este cartel es horrendo"; "que lo haga silencioso, diga instalando… y se
// reinicie"): a floating card (shadow, no border) with a title and one quiet line on the left,
// compact actions on the right and a hairline progress bar along its bottom edge. The install
// runs with no installer window; Velo closes and the installer opens it again when done.
Item {
    id: root
    objectName: "updateNotice"
    readonly property string phase: Backend.Updater.state
    readonly property bool wanted: phase === "available" || phase === "downloading" || phase === "ready" || phase === "error"
    // Same guard as Toast: no movement when minimised, in Performance mode or with reduced motion.
    readonly property var window: root.Window.window
    readonly property bool effectsAllowed: Theme.interactionAllowed && Theme.motion.scale > 0
        && (!window || (window.visibility !== Window.Minimized && window.visibility !== Window.Hidden))
    // `shown` outlives `wanted` until the exit has finished (same enter/leave edge as Toast).
    property bool shown: false
    visible: shown
    onWantedChanged: {
        if (wanted) {
            outgoing.stop(); shown = true
            if (effectsAllowed) { card.opacity = 0; card.y = -card.height - root.y; incoming.restart() }
            else { incoming.stop(); card.opacity = 1; card.y = 0 }
        } else if (shown) {
            incoming.stop()
            if (effectsAllowed) outgoing.restart()
            else { outgoing.stop(); shown = false; card.opacity = 1; card.y = 0 }
        }
    }
    onEffectsAllowedChanged: if (!effectsAllowed) {
        incoming.stop(); outgoing.stop(); card.y = 0; card.opacity = 1
        if (!wanted) shown = false
    }
    Component.onCompleted: if (wanted) shown = true
    width: Math.min(parent.width - 2 * Theme.space.gutter, Theme.toast.width)
    height: card.height
    Accessible.role: Accessible.AlertMessage
    Accessible.name: title.text + ". " + detail.text

    RectangularShadow {
        anchors.fill: card
        radius: card.radius
        offset.y: Theme.elevation.popover.offsetY
        blur: Theme.elevation.popover.blur
        spread: Theme.elevation.popover.spread
        color: Theme.elevation.popoverShadow
    }
    Rectangle {
        id: card
        objectName: "updateCard"
        width: parent.width
        height: Math.max(texts.height, actions.visible ? actions.height : 0) + 2 * Theme.space.md
        radius: Theme.radius.lg
        color: Theme.surface2
        clip: true

        Column {
            id: texts
            x: Theme.space.lg
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width - 2 * Theme.space.lg - (actions.visible ? actions.width + Theme.space.md : 0)
            spacing: Theme.space.xxs
            LibraryLabel {
                id: title
                objectName: "updateMessage"
                width: parent.width
                elide: Text.ElideRight
                textFormat: Text.PlainText
                font.weight: Theme.type.semibold
                color: root.phase === "error" ? Theme.danger : Theme.text
                text: root.phase === "error" ? qsTr("No se pudo actualizar")
                    : root.phase === "downloading" ? qsTr("Descargando Velo %1").arg(Backend.Updater.latestVersion)
                    : root.phase === "ready" ? qsTr("Instalando…")
                    : qsTr("Velo %1 está listo").arg(Backend.Updater.latestVersion)
                Accessible.ignored: true
            }
            LibraryLabel {
                id: detail
                width: parent.width
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
                textFormat: Text.PlainText
                color: Theme.textSecondary
                font.pixelSize: Theme.type.caption
                font.features: Theme.tabularFeatures
                text: root.phase === "error" ? Backend.Updater.error
                    : root.phase === "downloading" ? qsTr("%1 %").arg(Math.round(Backend.Updater.progress * 100))
                    : root.phase === "ready" ? qsTr("Velo se reiniciará solo en unos segundos.")
                    : qsTr("Se instala en segundo plano y Velo se reinicia solo.")
                Accessible.ignored: true
            }
        }

        Row {
            id: actions
            visible: root.phase === "available" || root.phase === "error"
            anchors.right: parent.right
            anchors.rightMargin: Theme.space.md
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.xs
            LibraryButton {
                objectName: "updateDismiss"
                text: qsTr("Más tarde")
                onClicked: Backend.Updater.dismiss()
            }
            LibraryButton {
                objectName: "updateInstall"
                primary: true
                text: root.phase === "error" ? qsTr("Reintentar") : qsTr("Actualizar")
                onClicked: Backend.Updater.install()
            }
        }

        // Hairline progress along the bottom edge while downloading (scale, no relayout).
        Rectangle {
            visible: root.phase === "downloading" || root.phase === "ready"
            anchors.bottom: parent.bottom
            width: parent.width
            height: Theme.stroke.focus
            color: Theme.accent
            transformOrigin: Item.Left
            scale: root.phase === "ready" ? 1 : Math.max(0, Math.min(1, Backend.Updater.progress))
            // The bar chases the value instead of jumping between discrete download steps.
            Behavior on scale { ScaleAnimator { duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
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
        onFinished: { root.shown = false; card.y = 0; card.opacity = 1 }
    }
}
