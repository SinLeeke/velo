import QtQuick
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"

// Switch (ref_02: green pill with a round knob). Space/Enter toggle when focused.
// `text` is the accessible name and must be a qsTr() string.
// Off: surface3 track with a controlBorder edge (>= 3:1 on the card), so the switch is findable
// without its knob; on: accent track. Disabled: whole control at Theme.library.disabledOpacity.
T.AbstractButton {
    id: control

    implicitWidth: Theme.control.toggleWidth
    implicitHeight: Theme.control.toggleHeight
    checkable: true
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    padding: 0
    opacity: enabled ? 1 : Theme.library.disabledOpacity

    Accessible.name: text
    Accessible.role: Accessible.CheckBox
    Accessible.checkable: true
    Accessible.checked: checked

    // Space toggles the focused switch (T.AbstractButton), ahead of any window Shortcut.
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    Keys.onReturnPressed: toggle()
    Keys.onEnterPressed: toggle()

    readonly property bool live: enabled && hovered
    // Toggle: the knob slides (XAnimator, render thread, fast, easeInOut) while the on/off fills
    // and knob inks crossfade (OpacityAnimator, fast). One shot per click; a second click
    // retargets mid-way. Transform and opacity only.
    background: Item {
        Rectangle {   // off track
            anchors.fill: parent
            radius: height / 2
            color: control.live ? Theme.surface2 : Theme.surface3
            border.width: Theme.stroke.hairline
            border.color: Theme.controlBorder
        }
        Rectangle {   // on track
            anchors.fill: parent
            radius: height / 2
            color: control.live ? Theme.accentHover : Theme.accent
            opacity: control.checked ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }
        }
        Rectangle {   // keyboard focus ring, outside the track with a gap (reads on the accent track too)
            anchors.fill: parent
            anchors.margins: -2 * Theme.stroke.focus
            radius: height / 2
            visible: control.visualFocus
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: Theme.focusRing
        }
    }

    contentItem: Item {
        Item {
            id: knob
            width: control.height - 2 * Theme.control.toggleKnobInset
            height: width
            y: Theme.control.toggleKnobInset
            x: control.checked ? control.width - width - Theme.control.toggleKnobInset : Theme.control.toggleKnobInset
            Behavior on x {
                XAnimator { duration: Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }
            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: Theme.textSecondary
            }
            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: Theme.accentInk
                opacity: control.checked ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                }
            }
            // Regla 7: on is confirmed by shape too, not only by colour: a check mark in the knob.
            // It grows in from dotFrom when the switch turns ON; turning OFF it only fades.
            IconImage {
                anchors.centerIn: parent
                source: Qt.resolvedUrl("../../icons/check.svg")
                sourceSize: Qt.size(width, width)
                width: Math.round(parent.width * 0.7); height: width
                color: Theme.accent
                opacity: control.checked ? 1 : 0
                scale: control.checked ? 1 : Theme.feel.dotFrom
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                Behavior on scale {
                    enabled: control.checked
                    ScaleAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }
        }
    }
}
