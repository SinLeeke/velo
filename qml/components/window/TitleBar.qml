import QtQuick
import QtQuick.Controls.impl
import "../../theme"
import Velo.Backend as Backend

// Transparent title bar strip laid over the window content.
// On Windows, dragging, double-click to maximize, Snap and the system menu come from the
// native hit test (WM_NCHITTEST → HTCAPTION over this strip, crates/app/cpp/window_chrome.cpp).
// Anything inside the strip that takes clicks stays clickable: MouseArea, Controls, Flickable,
// or any item declaring `property bool veloNoDrag: true` (e.g. items using only TapHandler).
// The rest of the strip drags the window, including the area over the content below it.
Item {
    id: bar

    property string controlsStyle: "mac"     // "mac" | "windows"
    property color ink: Theme.text           // glyph color for the Windows-style buttons
    // T8: secondary tone of the strip (sleep-timer indicator). Main binds the view's own
    // measured secondary ink (textSecondary / coverInkSecondary): an opacity over `ink` fell
    // under the alpha Rust checks for the light cover veil.
    property color inkSecondary: Theme.textSecondary
    // T8: distance from the right edge the view keeps for its own controls in this strip (the
    // library's Ajustes/tema buttons, Now Playing's tool pill). The timer sits left of them.
    property real trailingReserve: 0

    // `var`: under qmltestrunner the window is a QQuickView, which a `Window` property rejects.
    readonly property var window: Window.window
    readonly property real leadingInset: lights.visible ? lights.x + lights.width + Theme.windowControls.trafficGap / 2 + Theme.space.sm : 0
    readonly property real trailingInset: buttons.visible ? buttons.width : 0
    // Width the timer takes (0 while hidden), so the view can keep its own items clear of it.
    readonly property real timerWidth: timer.visible ? timer.width : 0

    // G5: through the native command (WM_SYSCOMMAND on Windows), so the system animates it.
    function toggleMaximized() {
        if (window)
            Backend.Desktop.toggleMaximizeWindow()
    }

    // Fallback for platforms without the native chrome (Linux, later). On Windows the strip is
    // non-client area, so these handlers never see a press there.
    DragHandler {
        enabled: Qt.platform.os !== "windows"
        target: null
        onActiveChanged: if (active && bar.window) bar.window.startSystemMove()
    }
    TapHandler {
        enabled: Qt.platform.os !== "windows"
        onDoubleTapped: bar.toggleMaximized()
    }

    TrafficLights {
        id: lights
        visible: bar.controlsStyle === "mac"
        x: Theme.windowControls.trafficHitSlop
        y: Theme.windowControls.trafficHitSlop
    }

    WindowButtons {
        id: buttons
        visible: bar.controlsStyle === "windows"
        anchors.right: parent.right
        anchors.top: parent.top
        height: parent.height
        ink: bar.ink
    }

    // T2: sleep-timer indicator. Not interactive: the strip keeps dragging the window.
    // T8: left of the view's own controls (trailingReserve), never under them.
    Row {
        id: timer
        objectName: "titleSleepTimer"
        readonly property bool active: Backend.Actions.sleepMinutes !== 0
        property int remainingMin: 1
        readonly property string label: Backend.Actions.sleepMinutes < 0
            ? qsTr("Al terminar la canción") : qsTr("%1 min").arg(remainingMin)
        visible: active
        anchors.right: parent.right
        anchors.rightMargin: Math.max(bar.trailingInset + Theme.space.md, bar.trailingReserve)
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.titleTimer.gap
        Accessible.role: Accessible.StaticText
        Accessible.name: Backend.Actions.sleepMinutes < 0 ? qsTr("Temporizador: al terminar la canción")
                                                          : qsTr("Temporizador: quedan %1 min").arg(remainingMin)

        function recalc() {
            const ends = Backend.Actions.sleepEndsAtMs
            if (ends <= 0) return
            const left = ends - Date.now()
            remainingMin = Math.max(1, Math.ceil(left / 60000))
            // Next change of the rounded value: when `left` drops to (remainingMin - 1) minutes.
            tick.interval = Math.max(1000, left - (Math.ceil(left / 60000) - 1) * 60000 + 50)
        }
        Connections {
            target: Backend.Actions
            function onSleepEndsAtMsChanged() { timer.recalc() }
        }
        Connections {
            target: Theme
            function onWindowVisibleChanged() { timer.recalc() }
            function onPlayingChanged() { timer.recalc() }
        }
        Component.onCompleted: recalc()

        Timer {
            id: tick
            repeat: true
            running: Backend.Actions.sleepEndsAtMs > 0 && Theme.windowVisible && Theme.playing
            onTriggered: timer.recalc()
        }
        IconImage {
            anchors.verticalCenter: parent.verticalCenter
            source: "../../icons/schedule.svg"
            sourceSize: Qt.size(Theme.titleTimer.icon, Theme.titleTimer.icon)
            color: bar.inkSecondary
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: timer.label
            color: bar.inkSecondary
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.features: Theme.tabularFeatures
        }
    }
}
