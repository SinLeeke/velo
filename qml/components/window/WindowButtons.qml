import QtQuick
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"

// Windows-style caption buttons (minimize, maximize/restore, close), top right.
// Chosen with Main.controlsStyle === "windows" (Settings.windowControls).
// Snap Layouts (Windows 11): the maximize button's rectangle, in logical scene coordinates, is
// reported to the native hit test (Backend.Desktop.setMaximizeButtonRect), which answers
// HTMAXBUTTON over it so hovering shows the Snap flyout. Reported on creation and whenever its
// geometry or the zoom changes (bindings, no Timer); a zero rectangle while these buttons are
// hidden (traffic lights) or gone.
Row {
    id: buttons

    property color ink: Theme.text

    readonly property var window: Window.window
    readonly property bool maximized: window ? window.visibility === Window.Maximized : false
    readonly property bool active: window ? window.active : true

    property Item maximizeButton: null
    function reportMaximizeButton() {
        if (maximizeButton) maximizeButton.reportRect()
        else Backend.Desktop.setMaximizeButtonRect(0, 0, 0, 0)
    }
    onXChanged: reportMaximizeButton()
    onYChanged: reportMaximizeButton()
    onVisibleChanged: reportMaximizeButton()
    Connections {
        target: buttons.parent
        function onXChanged() { buttons.reportMaximizeButton() }
        function onYChanged() { buttons.reportMaximizeButton() }
    }
    Connections {
        target: Theme
        function onZoomFactorChanged() { Qt.callLater(buttons.reportMaximizeButton) }
    }
    Connections {
        target: buttons.window
        function onVisibilityChanged() { buttons.reportMaximizeButton() }
    }
    Component.onDestruction: Backend.Desktop.setMaximizeButtonRect(0, 0, 0, 0)

    function trigger(kind) {
        if (!window)
            return
        if (kind === "close")
            window.close()
        else if (kind === "minimize")
            Backend.Desktop.minimizeWindow()        // G5: WM_SYSCOMMAND, so Windows animates it
        else
            Backend.Desktop.toggleMaximizeWindow()  // maximize or restore, animated likewise
    }

    Repeater {
        model: ["minimize", "zoom", "close"]

        delegate: Rectangle {
            id: button

            required property string modelData
            readonly property bool isClose: modelData === "close"
            function reportRect() {
                if (modelData !== "zoom") return
                if (!visible || !buttons.visible || width <= 0 || height <= 0
                        || (buttons.window && (buttons.window.visibility === Window.Hidden || buttons.window.visibility === Window.Minimized))) {
                    Backend.Desktop.setMaximizeButtonRect(0, 0, 0, 0)
                    return
                }
                const p = button.mapToItem(null, 0, 0)
                Backend.Desktop.setMaximizeButtonRect(p.x, p.y, button.width, button.height)
            }
            onXChanged: reportRect()
            onYChanged: reportRect()
            onWidthChanged: reportRect()
            onHeightChanged: reportRect()
            onVisibleChanged: reportRect()
            Component.onCompleted: if (modelData === "zoom") {
                buttons.maximizeButton = button
                Qt.callLater(reportRect)
            }
            Component.onDestruction: if (modelData === "zoom") Backend.Desktop.setMaximizeButtonRect(0, 0, 0, 0)

            width: Theme.windowControls.buttonWidth
            height: buttons.height
            color: Theme.library.clearPaint
            // Hover fill on its own layer so it fades (opacity: in at `instant`, out at `fast`);
            // the pressed shade swaps at once inside it.
            Rectangle {
                anchors.fill: parent
                color: button.isClose ? (area.pressed ? Theme.windowControls.closePress : Theme.windowControls.closeHover)
                     : (area.pressed ? Qt.rgba(buttons.ink.r, buttons.ink.g, buttons.ink.b, Theme.windowControls.pressAlpha) : Qt.rgba(buttons.ink.r, buttons.ink.g, buttons.ink.b, Theme.windowControls.hoverAlpha))
                opacity: area.containsMouse ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, area.containsMouse ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }

            property bool veloNoDrag: true
            activeFocusOnTab: true
            // Mouse focus shows no ring (like Control.visualFocus); Tab shows it.
            property bool mouseFocus: false
            onActiveFocusChanged: if (!activeFocus) mouseFocus = false
            Keys.onShortcutOverride: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter)
                    event.accepted = true
            }
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (!event.isAutoRepeat) buttons.trigger(modelData)
                    event.accepted = true
                }
            }
            Rectangle {
                anchors.fill: parent
                anchors.margins: Theme.stroke.focus
                radius: Theme.radius.xs
                color: Theme.library.clearPaint
                border.width: Theme.stroke.focus
                border.color: buttons.ink
                visible: button.activeFocus && !button.mouseFocus
            }

            Accessible.role: Accessible.Button
            Accessible.name: modelData === "close" ? qsTr("Cerrar")
                           : modelData === "minimize" ? qsTr("Minimizar")
                           : buttons.maximized ? qsTr("Restaurar") : qsTr("Maximizar")
            Accessible.onPressAction: buttons.trigger(modelData)

            MouseArea {
                id: area
                anchors.fill: parent
                hoverEnabled: true
                onPressed: { button.mouseFocus = true; button.forceActiveFocus(Qt.MouseFocusReason) }
                onClicked: buttons.trigger(button.modelData)
            }

            IconImage {
                anchors.centerIn: parent
                source: button.modelData === "close" ? "../../icons/close.svg"
                      : button.modelData === "minimize" ? "../../icons/remove.svg"
                      : buttons.maximized ? "../../icons/filter_none.svg" : "../../icons/crop_square.svg"
                sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                color: button.isClose && area.containsMouse ? Theme.windowControls.closeInk : buttons.ink
                opacity: buttons.active || area.containsMouse ? 1 : Theme.windowControls.inactiveOpacity
            }
        }
    }
}
