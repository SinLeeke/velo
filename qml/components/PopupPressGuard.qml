import QtQuick
import QtQuick.Templates as T

// T4b: tells whether the press being delivered right now is the one that closes an open popup.
// Qt lets a press outside a non-modal popup (menus, the device picker, the output and volume
// panels) through to the items below once it has closed the popup; the window's overlay sees
// that press first and emits pressed(). A handler reads `closing` when its own press arrives and
// keeps that answer until its release, so the closing click never counts as a click of its own.
// The flag lives only for that press's event (cleared on the next event loop turn).
// Usage: PopupPressGuard { id: guard; overlay: someItem.T.Overlay.overlay }
QtObject {
    id: guard

    property Item overlay: null
    property bool closing: false

    function clear() { closing = false }

    readonly property Connections watch: Connections {
        target: guard.overlay
        function onPressed() {
            guard.closing = true
            Qt.callLater(guard.clear)
        }
    }
}
