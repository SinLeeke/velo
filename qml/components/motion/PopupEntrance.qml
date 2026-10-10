import QtQuick
import QtQuick.Templates as T
import Velo.Backend
import "../../theme"

// S1-2: entrance of a Popup or Menu. Declare it as a property of the popup
// (`property PopupEntrance entrance: PopupEntrance { popup: root }`) and keep `enter: null`.
// Why not an `enter` Transition: it would hold `opened` false until it ends, delaying the focus
// hand-off in onOpened and the keyboard paths that check `opened`. Instead, once the popup is
// open, render-thread Animators run on the popup's own item (the parent of its contentItem):
// opacity 0 -> 1 and scale `fromScale` -> 1 around the popup's transformOrigin, ease-out.
// One shot; closing stops it. With Theme.motion at 0 nothing runs.
ParallelAnimation {
    id: entrance

    property T.Popup popup: null
    property real fromScale: Theme.entrance.dialogScale
    property int duration: Theme.motion.base
    readonly property Item item: popup && popup.contentItem ? popup.contentItem.parent : null
    readonly property bool effectsOn: Player.isPlaying && item !== null && item.Window.window !== null
        && item.Window.visibility !== Window.Minimized && item.Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || item.Window.active)
    onEffectsOnChanged: if (!effectsOn && running) complete()

    property Connections hook: Connections {
        target: entrance.popup
        // An exit transition (or a stopped entrance) may have left the item faded or scaled.
        function onAboutToShow() {
            if (entrance.item) {
                entrance.item.opacity = 1
                entrance.item.scale = 1
            }
        }
        function onOpened() {
            if (entrance.effectsOn && entrance.duration > 0 && entrance.item)
                entrance.restart()
        }
        function onAboutToHide() { entrance.stop() }
    }

    OpacityAnimator {
        target: entrance.item
        from: 0
        to: 1
        duration: Math.max(1, entrance.duration)
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeOut
    }
    ScaleAnimator {
        target: entrance.item
        from: entrance.fromScale
        to: 1
        duration: entrance.duration
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeOut
    }
}
