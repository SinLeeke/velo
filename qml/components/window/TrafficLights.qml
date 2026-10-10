import QtQuick
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"

// macOS-style window controls: close, minimize, maximize/restore.
// Glyphs show while the pointer is over the group (as on macOS); the lights turn neutral when
// the window is inactive, and regain their color on hover.
Item {
    id: lights

    readonly property var window: Window.window
    readonly property bool active: window ? window.active : true
    readonly property bool maximized: window ? window.visibility === Window.Maximized : false
    readonly property bool lit: active || groupHover.hovered

    // The whole group, gaps included, is client area for the native hit test, so hover does
    // not flicker while the pointer crosses between lights.
    property bool veloNoDrag: true

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

    width: row.width
    height: Theme.windowControls.trafficDiameter

    HoverHandler {
        id: groupHover
    }

    Row {
        id: row
        spacing: Theme.windowControls.trafficGap

        Repeater {
            model: ["close", "minimize", "zoom"]

            delegate: Rectangle {
                id: light

                required property string modelData
                readonly property var colors: modelData === "close" ? Theme.windowControls.trafficClose : modelData === "minimize" ? Theme.windowControls.trafficMinimize : Theme.windowControls.trafficZoom

                width: Theme.windowControls.trafficDiameter
                height: Theme.windowControls.trafficDiameter
                radius: width / 2
                color: !lights.lit ? Theme.windowControls.trafficIdle
                     : area.pressed ? Qt.darker(colors.fill, Theme.windowControls.pressDarken)
                     : colors.fill
                border.width: Theme.windowControls.trafficRing
                border.color: lights.lit ? colors.ring : Theme.windowControls.trafficIdleRing
                // Color states switch immediately; only the glyph's opacity animates below.

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
                        if (!event.isAutoRepeat) lights.trigger(modelData)
                        event.accepted = true
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -Theme.stroke.focus
                    radius: width / 2
                    color: Theme.library.clearPaint
                    border.width: Theme.stroke.focus
                    border.color: Theme.text
                    visible: light.activeFocus && !light.mouseFocus
                }

                Accessible.role: Accessible.Button
                Accessible.name: modelData === "close" ? qsTr("Cerrar")
                               : modelData === "minimize" ? qsTr("Minimizar")
                               : lights.maximized ? qsTr("Restaurar") : qsTr("Maximizar")
                Accessible.onPressAction: lights.trigger(modelData)

                // The click target is larger than the 12 px disc (Theme.windowControls.trafficHitSlop
                // up and down, half the gap sideways), without changing the drawing.
                MouseArea {
                    id: area
                    anchors.fill: parent
                    anchors.topMargin: -Theme.windowControls.trafficHitSlop
                    anchors.bottomMargin: -Theme.windowControls.trafficHitSlop
                    anchors.leftMargin: -Theme.windowControls.trafficGap / 2
                    anchors.rightMargin: -Theme.windowControls.trafficGap / 2
                    onPressed: { light.mouseFocus = true; light.forceActiveFocus(Qt.MouseFocusReason) }
                    onClicked: lights.trigger(light.modelData)
                }

                IconImage {
                    anchors.centerIn: parent
                    source: light.modelData === "close" ? "../../icons/close.svg"
                          : light.modelData === "minimize" ? "../../icons/remove.svg"
                          : lights.maximized ? "../../icons/filter_none.svg" : "../../icons/crop_square.svg"
                    sourceSize: Qt.size(Theme.windowControls.trafficGlyph, Theme.windowControls.trafficGlyph)
                    color: light.colors.glyph
                    // Glyphs fade in/out with the group hover or focus (opacity, `fast`).
                    readonly property bool shown: (groupHover.hovered || light.activeFocus) && lights.lit
                    opacity: shown ? 1 : 0
                    Behavior on opacity {
                        OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                }
            }
        }
    }
}
