import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"
import ".."

// Hover opens the slider; clicking/Enter/Space toggles mute. Wheel over the button changes the
// volume; Up/Down step it while the button has focus, Esc folds the panel.
// G4 (ronda 2, «la barra de volumen a veces queda persistente»): the panel only stays for what
// the user is doing right now, never for a leftover focus:
//  - mouse: open while the pointer is on the button or the panel, plus a small slop around both
//    (`volumeHoverSlop`), and while the slider is held; leaving that area folds it at once, and
//    so does releasing the slider outside it.
//  - keyboard (Tab focus, Up/Down, showSlider()): open until focus leaves both controls, Esc, or
//    the mouse passes over and leaves.
//  - a press anywhere outside the button and the panel folds it (Popup closePolicy).
//  - the window losing activation (Alt+Tab, another app) folds it too.
// The panel is a non-modal Popup so it draws above the docked queue panel (Main, z 2), which
// otherwise covered it. No Timer bridges anything: the hover areas touch.
Item {
    id: root

    readonly property int step: Theme.sliderInput.volumeStep
    // B1: `volume` is what the controls show and accumulate on: the user's own latest value
    // (`pending`) while the backend has not confirmed it, else what the backend reports. A stale
    // echo of an older value never reaches the knob, on release either. The "last audible volume"
    // is Player.lastAudibleVolume (no copy here).
    readonly property int observed: Backend.Player.volume
    property int pending: -1
    readonly property int volume: pending >= 0 ? pending : observed
    readonly property bool canChange: Backend.Player.canControl
    readonly property var window: root.Window.window
    readonly property bool settleAllowed: Backend.Player.isPlaying && root.visible
        && (!window || (window.visible && window.visibility !== Window.Minimized))
    onSettleAllowedChanged: if (!settleAllowed) {
        settle.stop()
        if (!slider.pressed) pending = -1
    }
    // Followed again once the backend reports the same value (never mid-drag).
    onObservedChanged: if (!slider.pressed && pending >= 0 && observed === pending) { pending = -1; settle.stop() }
    // Gives up on a value the backend never confirms (rejected, clamped). Runs only for the
    // 1.5 s after a change, matching the bridge's optimistic hold; nothing runs at rest.
    Timer { id: settle; objectName: "volumeSettle"; interval: Theme.library.volumeSettle; onTriggered: if (!slider.pressed) root.pending = -1 }
    property bool dismissed: false
    property bool hoverOpen: false       // the pointer came over the button and has not left the area
    property bool keyOpen: false         // opened from the keyboard (or showSlider())
    readonly property bool open: canChange && !dismissed && root.visible
                                 && (hoverOpen || keyOpen || slider.pressed)

    function toggleMute() {
        setVolume(volume > 0 ? 0 : Backend.Player.lastAudibleVolume)
    }
    function showSlider() { dismissed = false; keyOpen = true; button.forceActiveFocus(Qt.TabFocusReason) }
    function closeSlider() { button.forceActiveFocus(Qt.OtherFocusReason); keyOpen = false; dismissed = true }
    // Called (deferred, after the hover event settles) whenever one of the hover areas is left.
    function checkHover() {
        if (slider.pressed || buttonHover.hovered || slider.hovered || buttonZone.hovered || panelZone.hovered
                || buttonSlop.containsMouse || panelSlop.containsMouse)
            return
        hoverOpen = false
        keyOpen = false
    }
    function checkFocus() {
        if (!button.activeFocus && !slider.activeFocus)
            keyOpen = false
    }

    // Sent at every change (the backend coalesces); wheel and keys accumulate on `volume`.
    function setVolume(v) {
        const next = Math.max(0, Math.min(100, Math.round(v)))
        pending = next
        if (settleAllowed) settle.restart()
        Backend.Player.setVolume(next)
    }

    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight
    // The button's share of the hover area: the button plus the slop around it. The zone is the
    // button's parent so its HoverHandler sees the pointer over the button too (hover goes to the
    // top-most item and its ancestors, not to siblings underneath).
    Item {
        id: buttonZoneItem
        anchors.fill: parent
        anchors.margins: -Theme.audioOutput.volumeHoverSlop
        HoverHandler { id: buttonZone; onHoveredChanged: if (!hovered) Qt.callLater(root.checkHover) }
        // The slop itself: an empty area only reports hover through an item that accepts it.
        MouseArea {
            id: buttonSlop
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
            onContainsMouseChanged: if (!containsMouse) Qt.callLater(root.checkHover)
        }

        IconButton {
            id: button
            anchors.fill: parent
            anchors.margins: Theme.audioOutput.volumeHoverSlop
            objectName: "volumeMuteToggle"
            text: root.volume === 0 ? qsTr("Quitar silencio") : qsTr("Silenciar")
            Accessible.description: qsTr("Volumen: %1%").arg(root.volume)
            Accessible.checkable: true
            Accessible.checked: root.volume === 0
            iconSource: root.volume === 0 ? Qt.resolvedUrl("../../icons/volume_off.svg")
                      : root.volume < 50 ? Qt.resolvedUrl("../../icons/volume_down.svg")
                      : Qt.resolvedUrl("../../icons/volume_up.svg")
            active: root.open
            enabled: root.canChange
            onClicked: {
                root.toggleMute()
                if (!buttonHover.hovered) root.showSlider() // touch: open until a tap outside
            }
            // Only keyboard focus (Tab) opens it; focus left behind by the mouse never pins it.
            onActiveFocusChanged: {
                if (activeFocus) root.dismissed = false
                else Qt.callLater(root.checkFocus)
            }
            onVisualFocusChanged: if (visualFocus) root.keyOpen = true

            Keys.onShortcutOverride: event => {
                if ([Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter, Qt.Key_Up, Qt.Key_Down, Qt.Key_Escape].indexOf(event.key) >= 0) event.accepted = true
            }
            Keys.onReturnPressed: root.toggleMute()
            Keys.onEnterPressed: root.toggleMute()
            Keys.onSpacePressed: root.toggleMute()
            Keys.onUpPressed: { root.dismissed = false; root.keyOpen = true; root.setVolume(root.volume + root.step) }
            Keys.onDownPressed: { root.dismissed = false; root.keyOpen = true; root.setVolume(root.volume - root.step) }
            Keys.onEscapePressed: root.closeSlider()

            HoverHandler {
                id: buttonHover
                onHoveredChanged: {
                    if (hovered) { root.dismissed = false; root.hoverOpen = true }
                    else Qt.callLater(root.checkHover)
                }
            }
            WheelHandler {
                enabled: root.canChange
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: event => root.setVolume(root.volume + (event.angleDelta.y > 0 ? root.step : -root.step))
            }
        }
    }

    // The window losing activation folds the panel (a leftover hover or focus never pins it).
    readonly property bool windowActive: Window.active
    onWindowActiveChanged: if (!windowActive) { hoverOpen = false; keyOpen = false }

    // The popup opens with `open` and closes once the fold has played (panel at opacity 0), or at
    // once when the view goes away. Driven by `open` itself, not by the panel's opacity: a popup
    // closed from outside (press outside) leaves its hidden panel wherever its fade stopped, and
    // the next hover must still open it. Deferred: never open/close the popup from inside an
    // animation step.
    function syncPopup() {
        if (open && root.visible) {
            if (!popup.visible) popup.open()
        } else if (!root.visible || panel.opacity <= 0) {
            popup.close()
        }
    }
    onOpenChanged: Qt.callLater(syncPopup)
    onVisibleChanged: Qt.callLater(syncPopup)

    T.Popup {
        id: popup
        parent: button
        // The popup is the panel plus the hover slop on its top and sides (not below: there the
        // button's own zone takes over, and a click on the button must reach the button).
        x: Math.round((button.width - width) / 2)
        y: -height
        width: root.width + 2 * Theme.audioOutput.volumeHoverSlop
        height: Theme.audioOutput.volumePanelHeight + Theme.audioOutput.volumeHoverSlop
        padding: 0
        modal: false
        dim: false
        focus: false
        // A press outside the panel and outside its button folds it (the press still reaches
        // whatever was under it: the popup is not modal).
        closePolicy: T.Popup.CloseOnPressOutside | T.Popup.CloseOnPressOutsideParent
        enter: null
        exit: null
        background: null
        onClosed: if (root.open) { root.hoverOpen = false; root.keyOpen = false; root.dismissed = true }

        // Motion (S1-2): hover opens it, so it is quick: opacity 0 -> 1 and an 8 px rise
        // (Translate, never a height), `fast` ease-out; it folds back in `instant`, ease-in. The
        // popup stays open while the fold plays (`syncPopup`). A NumberAnimation, not an
        // Animator: the popup leaves the scene right after the fold, and a render-thread
        // Animator still finishing on a removed item crashes Qt 6.10.
        contentItem: Item {
            // The panel's share of the hover area (the panel's ancestor, so the slider counts).
            HoverHandler { id: panelZone; onHoveredChanged: if (!hovered) Qt.callLater(root.checkHover) }
            MouseArea {
                id: panelSlop
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
                onContainsMouseChanged: if (!containsMouse) Qt.callLater(root.checkHover)
            }

            Rectangle {
                id: panel
                anchors.fill: parent
                anchors.leftMargin: Theme.audioOutput.volumeHoverSlop
                anchors.rightMargin: Theme.audioOutput.volumeHoverSlop
                anchors.topMargin: Theme.audioOutput.volumeHoverSlop
                opacity: root.open ? 1 : 0
                onOpacityChanged: if (opacity <= 0 && !root.open) Qt.callLater(root.syncPopup)
                Behavior on opacity {
                    NumberAnimation {
                        duration: root.open ? Theme.motion.fast : Theme.motion.instant
                        easing.type: Easing.Bezier
                        easing.bezierCurve: root.open ? Theme.motion.easeOut : Theme.motion.easeIn
                    }
                }
                transform: Translate {
                    y: root.open ? 0 : Theme.entrance.popoverShift
                    Behavior on y {
                        NumberAnimation {
                            duration: root.open ? Theme.motion.fast : Theme.motion.instant
                            easing.type: Easing.Bezier
                            easing.bezierCurve: root.open ? Theme.motion.easeOut : Theme.motion.easeIn
                        }
                    }
                }
                radius: width / 2
                color: Theme.surface1
                border.width: Theme.stroke.hairline
                border.color: Theme.hairline

                RectangularShadow {
                    z: -1
                    anchors.fill: parent
                    radius: parent.radius
                    offset.y: Theme.elevation.popover.offsetY
                    blur: Theme.elevation.popover.blur
                    spread: Theme.elevation.popover.spread
                    color: Theme.audioOutput.shadow
                }

                T.Slider {
                    id: slider
                    objectName: "volumeVerticalSlider"
                    anchors.fill: parent
                    anchors.topMargin: Theme.space.lg
                    anchors.bottomMargin: Theme.space.lg
                    orientation: Qt.Vertical
                    from: 0
                    to: 100
                    stepSize: root.step
                    value: root.volume
                    focusPolicy: Qt.StrongFocus
                    Accessible.name: qsTr("Volumen")
                    Accessible.role: Accessible.Slider
                    Accessible.description: qsTr("Volumen: %1%").arg(root.volume)
                    onMoved: root.setVolume(value)
                    // The last order goes out on release, and the display follows the backend again
                    // only if it already agrees (otherwise `pending` holds until it does). Released
                    // outside the hover area, the panel folds.
                    onPressedChanged: if (!pressed) {
                        root.setVolume(value)
                        if (root.observed === root.pending) { root.pending = -1; settle.stop() }
                        Qt.callLater(root.checkHover)
                    }
                    onActiveFocusChanged: if (!activeFocus) Qt.callLater(root.checkFocus)
                    onHoveredChanged: if (!hovered) Qt.callLater(root.checkHover)
                    onVisualFocusChanged: if (visualFocus) root.keyOpen = true
                    Keys.onShortcutOverride: event => {
                        if ([Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter, Qt.Key_Up, Qt.Key_Down, Qt.Key_Escape].indexOf(event.key) >= 0) event.accepted = true
                    }
                    Keys.onReturnPressed: root.toggleMute()
                    Keys.onEnterPressed: root.toggleMute()
                    Keys.onSpacePressed: root.toggleMute()
                    Keys.onEscapePressed: root.closeSlider()

                    background: Item {
                        x: slider.leftPadding + slider.availableWidth / 2 - width / 2
                        y: slider.topPadding
                        width: Theme.stroke.progress
                        height: slider.availableHeight

                        Rectangle {   // remaining track
                            width: parent.width
                            height: slider.handle.y - slider.topPadding + slider.handle.height / 2
                            radius: width / 2
                            color: Theme.library.controlTrack
                        }
                        Rectangle {   // filled part, from the bottom
                            anchors.bottom: parent.bottom
                            width: parent.width
                            height: parent.height - (slider.handle.y - slider.topPadding + slider.handle.height / 2)
                            radius: width / 2
                            color: Theme.accent
                        }
                    }

                    handle: Rectangle {
                        x: slider.leftPadding + slider.availableWidth / 2 - width / 2
                        y: slider.topPadding + slider.visualPosition * (slider.availableHeight - height)
                        width: Theme.control.sliderKnobHeight
                        height: Theme.stroke.progress
                        radius: height / 2
                        color: slider.hovered || slider.pressed ? Theme.accentHover : Theme.accent
                        // R3 (P3-7): keyboard focus draws a ring around the knob with a gap of
                        // panel between them (a 2 px border on a 3 px knob did not show).
                        Rectangle {
                            anchors.centerIn: parent
                            width: parent.width + 4 * Theme.stroke.focus
                            height: parent.height + 4 * Theme.stroke.focus
                            radius: height / 2
                            visible: slider.visualFocus
                            color: Theme.library.clearPaint
                            border.width: Theme.stroke.focus
                            border.color: Theme.focusRing
                        }
                    }
                }
            }
        }
    }
}
