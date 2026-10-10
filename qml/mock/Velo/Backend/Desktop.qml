pragma Singleton
import QtQuick
QtObject {
    signal showRequested()
    function showWindow() {}
    // Ronda 2 (G5): native window commands (WM_SYSCOMMAND, so Windows animates them). The mock
    // records the last one and, when a preview sets `window`, applies it with the Qt calls.
    property var window: null
    property string lastWindowCommand: ""
    function minimizeWindow() {
        lastWindowCommand = "minimize"
        if (window) window.showMinimized()
    }
    // R5 Snap Layouts: the native hit test answers HTMAXBUTTON over this scene rectangle.
    property rect maximizeButtonRect: Qt.rect(0, 0, 0, 0)
    function setMaximizeButtonRect(x, y, width, height) { maximizeButtonRect = Qt.rect(x, y, width, height) }
    function toggleMaximizeWindow() {
        lastWindowCommand = "toggleMaximize"
        if (window) {
            if (window.visibility === Window.Maximized) window.showNormal()
            else window.showMaximized()
        }
    }
}
