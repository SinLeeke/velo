import QtQuick
import QtQuick.Shapes
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import QtQuick.Effects
import Velo.Backend as Backend
import "../theme"
import "../components/library"
import "../components"
import "../components/nowplaying/scallop.js" as Scallop

Rectangle {
    id: root
    color: Theme.bg
    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Iniciar sesión en Velo")
    property string errorMessage: ""
    property bool celebrating: false
    signal successFinished()
    property bool pendingSignIn: false
    property bool appeared: false
    readonly property bool waiting: Backend.Account.busy && !Backend.Account.signedIn
    readonly property var window: root.Window.window
    readonly property bool effectsAllowed: visible && Theme.effectsAllowed && Theme.motion.scale > 0
        && (!window || (window.visibility !== Window.Minimized && window.visibility !== Window.Hidden
            && (window.active || !Theme.pauseEffectsInBackground)))

    // Main conserva el gate entre creaciones del Loader: una sola entrada por apertura.
    function reveal(firstAppearance) {
        if (appeared) return
        appeared = true
        if (firstAppearance) { logoIn.begin(); titleIn.begin(); textIn.begin(); actionsIn.begin(); disclaimerIn.begin() }
        focusSignIn()
    }
    // Enter signs in from the start: the only action holds the focus (no ring until the
    // keyboard is used: OtherFocusReason is not visualFocus). Also when the wait ends.
    function focusSignIn() {
        if (loginButton.visible) loginButton.forceActiveFocus(Qt.OtherFocusReason)
    }
    onWaitingChanged: if (!waiting) Qt.callLater(focusSignIn)
    function signIn() {
        if (waiting || celebrating) return
        toast.dismiss()
        Backend.Account.signIn()
    }
    function celebrate() {
        if (celebrating) return
        celebrating = true; toast.dismiss()
        if (effectsAllowed) success.start()
        else Qt.callLater(() => root.successFinished())
    }
    onEffectsAllowedChanged: if (!effectsAllowed) {
        if (!appeared) return
        shake.stop(); shakeLayer.x = 0
        if (celebrating && success.running) { success.stop(); Qt.callLater(() => root.successFinished()) }
    }
    Connections {
        target: Backend.Account
        // What failed, in the backend's words, and a way to try again right there.
        function onSignInFailed(reason) {
            root.errorMessage = reason; toast.show(reason, qsTr("Volver a intentar"), "error")
            shake.stop(); shakeLayer.x = 0
            if (root.effectsAllowed) shake.start()
        }
        function onAudioFailed(reason) { toast.show(reason, "", "error") }
        function onBusyChanged() { if (Backend.Account.busy) { toast.dismiss(); root.errorMessage = "" } }
    }
    Toast { id: toast; objectName: "loginToast"; onActionRequested: root.signIn() }
    Item {
        id: shakeLayer
        width: parent.width; height: parent.height
    Flickable {
        id: viewport
        x: Theme.space.gutter; y: Theme.size.titleBar
        width: parent.width - 2 * x; height: parent.height - y
        // El aviso va al pie: el formulario se centra en el espacio que queda sobre él y,
        // en ventanas chicas, el contenido crece y se desplaza en vez de taparse.
        readonly property real footerBlock: disclaimer.height + Theme.space.xl + Theme.space.lg
        contentWidth: width
        contentHeight: Math.max(height, form.height + footerBlock + Theme.space.gutter)
        clip: true; boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        Column {
            id: form
            width: Math.min(Theme.login.formWidth, viewport.width)
            x: (viewport.width - width) / 2
            y: Math.max(Theme.space.gutter, (viewport.height - viewport.footerBlock - height) / 2)
            spacing: Theme.space.xl
            // Logo de Velo protagonista: el disco festoneado de la app (mismo lenguaje que el
            // botón de reproducir) con la marca teñida al rasterizar, y un halo SDF quieto.
            Column {
                id: brand
                width: parent.width; spacing: Theme.space.lg
                transform: Translate { id: logoShift }
                Item {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Theme.login.heroMark; height: width
                    RectangularShadow {
                        anchors.fill: parent
                        radius: width / 2
                        blur: Theme.elevation.cover.blur
                        spread: Theme.elevation.cover.spread
                        color: Theme.accentSoft
                    }
                    Shape {
                        id: mark
                        anchors.fill: parent
                        preferredRendererType: Shape.CurveRenderer
                        ShapePath {
                            strokeWidth: -1; fillColor: Theme.accentSoft
                            PathPolyline { path: Scallop.points(mark.width, mark.height, Theme.scallop.lobesPlay, Theme.scallop.depth) }
                        }
                    }
                    IconImage {
                        objectName: "veloLogo"
                        anchors.centerIn: parent
                        source: Qt.resolvedUrl("../icons/velo_mono.svg")
                        sourceSize: Qt.size(Theme.login.heroGlyph, Theme.login.heroGlyph)
                        color: Theme.accent
                    }
                }
                LibraryLabel {
                    width: parent.width; horizontalAlignment: Text.AlignHCenter
                    text: qsTr("Velo"); font.pixelSize: Theme.type.hero; font.weight: Theme.type.heavy
                    font.letterSpacing: Theme.type.tracking(Theme.type.hero)
                }
            }
            LibraryLabel {
                id: title
                width: parent.width; horizontalAlignment: Text.AlignHCenter
                text: qsTr("Tu música merece su espacio.")
                font.pixelSize: Theme.type.heading; font.weight: Theme.type.semibold
                font.letterSpacing: Theme.type.tracking(Theme.type.heading)
                wrapMode: Text.Wrap
                transform: Translate { id: titleShift }
            }
            LibraryLabel {
                id: explanation
                width: parent.width; horizontalAlignment: Text.AlignHCenter
                text: qsTr("Se abrirá Spotify en tu navegador; ahí puedes entrar con tu correo, Google, Apple o Facebook. Para escuchar música necesitas Spotify Premium.")
                color: Theme.textSecondary; wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
                transform: Translate { id: textShift }
            }
            Column {
                id: actions
                width: parent.width; spacing: Theme.space.md
                transform: Translate { id: actionsShift }
                // Una sola acción. Sin ícono de marca: el texto basta (la app personal vive en Ajustes).
                Item {
                    width: parent.width; height: Theme.login.buttonHeight
                    visible: !root.waiting
                    Rectangle {
                        id: halo; anchors.fill: parent; radius: Theme.radius.lg
                        color: Theme.accentSoft; opacity: 0; scale: 1
                    }
                    LibraryButton {
                        id: loginButton; objectName: "signInButton"
                        anchors.fill: parent; primary: true
                        text: root.celebrating ? qsTr("Todo listo") : qsTr("Iniciar sesión con Spotify")
                        iconSource: root.celebrating ? Qt.resolvedUrl("../icons/check.svg") : ""
                        onClicked: if (!root.celebrating) root.signIn()
                    }
                }
                // Indicador creado solo durante OAuth; entrada única y luego quieto.
                // No deja un spinner gastando CPU durante los minutos que dure el navegador.
                Loader {
                    width: parent.width; height: item ? item.implicitHeight : 0
                    active: root.waiting; visible: active
                    sourceComponent: Column {
                        id: waitingBody
                        width: actions.width; spacing: Theme.space.md
                        transform: Translate { id: waitingShift }
                        Row {
                            anchors.horizontalCenter: parent.horizontalCenter
                            spacing: Theme.space.md
                            IconImage {
                                anchors.verticalCenter: parent.verticalCenter
                                source: Qt.resolvedUrl("../icons/open_in_new.svg")
                                sourceSize: Qt.size(Theme.icon.lg, Theme.icon.lg); color: Theme.accent
                            }
                            LibraryLabel {
                                anchors.verticalCenter: parent.verticalCenter
                                text: qsTr("Esperando autorización…")
                                font.pixelSize: Theme.type.title; font.weight: Theme.type.semibold
                            }
                        }
                        LibraryLabel {
                            width: parent.width; horizontalAlignment: Text.AlignHCenter
                            text: qsTr("Termina el inicio de sesión en el navegador y vuelve a Velo. Autoriza con la cuenta que quieres usar aquí.")
                            color: Theme.textSecondary; wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
                        }
                        LibraryButton {
                            objectName: "cancelSignInButton"
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: qsTr("Cancelar"); iconSource: Qt.resolvedUrl("../icons/close.svg")
                            onClicked: Backend.Account.cancelSignIn()
                        }
                        FirstUseReveal { id: waitingIn; item: waitingBody; shift: waitingShift; effectsAllowed: root.effectsAllowed }
                        Component.onCompleted: waitingIn.begin()
                    }
                }
            }
        }
        // Aviso de no afiliación: al pie, discreto, nunca encima del formulario.
        LibraryLabel {
            id: disclaimer
            objectName: "loginDisclaimer"
            width: Math.min(Theme.library.readableWidth, viewport.width)
            x: (viewport.width - width) / 2
            y: viewport.contentHeight - height - Theme.space.lg
            horizontalAlignment: Text.AlignHCenter
            text: qsTr("Velo es un proyecto independiente y no está afiliado, asociado ni respaldado por Spotify. Spotify es una marca registrada de Spotify AB.")
            font.pixelSize: Theme.type.caption
            font.letterSpacing: Theme.type.tracking(Theme.type.caption)
            color: Theme.textTertiary; wrapMode: Text.Wrap; lineHeight: Theme.type.bodyLineHeight
            transform: Translate { id: disclaimerShift }
        }
    }
    }
    FirstUseReveal { id: logoIn; item: brand; shift: logoShift; order: 0; effectsAllowed: root.effectsAllowed }
    FirstUseReveal { id: titleIn; item: title; shift: titleShift; order: 1; effectsAllowed: root.effectsAllowed }
    FirstUseReveal { id: textIn; item: explanation; shift: textShift; order: 2; effectsAllowed: root.effectsAllowed }
    FirstUseReveal { id: actionsIn; item: actions; shift: actionsShift; order: 3; effectsAllowed: root.effectsAllowed }
    FirstUseReveal { id: disclaimerIn; item: disclaimer; shift: disclaimerShift; order: 4; effectsAllowed: root.effectsAllowed }
    // Éxito: 180 ms de pulso + 150 ms de salida + SnapshotFade (220 ms): ~550 ms.
    SequentialAnimation {
        id: success
        ParallelAnimation {
            SequentialAnimation {
                ScaleAnimator { target: loginButton; to: Theme.login.successPulse; duration: Theme.motion.instant; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                ScaleAnimator { target: loginButton; to: 1; duration: Theme.motion.instant; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            SequentialAnimation {
                ScriptAction { script: halo.opacity = 1 }
                ParallelAnimation {
                    ScaleAnimator { target: halo; to: Theme.login.successPulse; duration: 2 * Theme.motion.instant; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    OpacityAnimator { target: halo; to: 0; duration: Math.max(1, 2 * Theme.motion.instant); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }
        }
        ParallelAnimation {
            OpacityAnimator { target: shakeLayer; to: 0; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            ScaleAnimator { target: shakeLayer; to: Theme.login.exitScale; duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        onFinished: root.successFinished()
    }
    SequentialAnimation {
        id: shake
        XAnimator { target: shakeLayer; to: -Theme.login.shakeDistance; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        XAnimator { target: shakeLayer; to: Theme.login.shakeDistance; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        XAnimator { target: shakeLayer; to: -Theme.login.shakeDistance; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        XAnimator { target: shakeLayer; to: Theme.login.shakeDistance; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        XAnimator { target: shakeLayer; to: -Theme.login.shakeDistance; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        XAnimator { target: shakeLayer; to: 0; duration: Theme.motion.shakeStep; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
}
