import QtQuick
import Velo.Backend
import "../../theme"

// S1-2: how a Popup leaves (its `exit`): shorter than the entrance, ease-in, from wherever the
// entrance got to (no `from`, so closing mid-entrance does not jump). Opacity -> 0 and
// scale -> `toScale`. NumberAnimations, not Animators: a Popup is not an Item, and a Transition
// animates the popup's own properties. With Theme.motion at 0 it completes at once.
Transition {
    id: transition
    property real toScale: Theme.entrance.dialogExitScale
    property bool effectsOn: Player.isPlaying
    property int duration: effectsOn ? Theme.motion.fast : 0

    NumberAnimation {
        property: "opacity"
        to: 0
        duration: transition.duration
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeIn
    }
    NumberAnimation {
        property: "scale"
        to: transition.toScale
        duration: transition.duration
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeIn
    }
}
