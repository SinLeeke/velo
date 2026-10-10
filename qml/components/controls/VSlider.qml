import QtQuick
import QtQuick.Templates as T
import "../../theme"

// Horizontal slider (ref_02: green fill, dark track, vertical bar knob). Arrow keys step,
// Home/End jump (built into T.Slider). Bind `value`; write it back from onMoved.
// `text` is the accessible name and must be a qsTr() string.
T.Slider {
    id: control

    property string text: ""

    implicitWidth: Theme.size.coverMax
    implicitHeight: Math.max(Theme.size.iconButton, Theme.size.minimumHitTarget)
    focusPolicy: Qt.StrongFocus
    hoverEnabled: true
    padding: 0

    Accessible.name: text
    Accessible.role: Accessible.Slider
    opacity: enabled ? 1 : Theme.library.disabledOpacity

    background: Item {
        x: control.leftPadding
        y: control.topPadding + control.availableHeight / 2 - height / 2
        width: control.availableWidth
        height: Theme.control.sliderKnobHeight

        Rectangle {   // remaining track: controlBorder, >= 3:1 on the card (WCAG 1.4.11)
            anchors.verticalCenter: parent.verticalCenter
            x: control.handle.x + control.handle.width / 2
            width: parent.width - x
            height: Theme.stroke.progress
            radius: height / 2
            color: Theme.controlBorder
        }
        Rectangle {   // filled part
            anchors.verticalCenter: parent.verticalCenter
            width: control.handle.x + control.handle.width / 2
            height: Theme.stroke.progress
            radius: height / 2
            color: control.enabled ? Theme.accent : Theme.textDisabled
        }
    }

    handle: Rectangle {
        x: control.leftPadding + control.visualPosition * (control.availableWidth - width)
        y: control.topPadding + control.availableHeight / 2 - height / 2
        width: Theme.stroke.progress
        height: Theme.control.sliderKnobHeight
        radius: width / 2
        color: control.enabled ? (control.hovered || control.pressed ? Theme.accentHover : Theme.accent) : Theme.textDisabled
        border.width: control.visualFocus ? Theme.stroke.focus : 0
        border.color: Theme.text
        // The knob grows while held (transform scale: instant up, fast back).
        scale: control.pressed ? Theme.feel.knobGrow : 1
        Behavior on scale {
            ScaleAnimator { duration: control.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
    }
}
