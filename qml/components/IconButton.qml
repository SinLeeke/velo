import QtQuick
import QtQuick.Templates as T
import QtQuick.Controls.impl
import QtQuick.Effects
import "../theme"
import "library"

// Generic icon-only button (Material Symbols Rounded SVG tinted by IconImage, no shader).
// Defaults suit the neutral library world; the Now Playing surface passes cover.* colors.
// `text` is the accessible name and must be a qsTr() string; it is also the tooltip shown on
// hover or keyboard focus (Windows convention for icon-only buttons). `toolTip: ""` hides it.
// `toggle: true` when `active` is an on/off state (shuffle, repeat, lyrics): screen readers
// then announce it as a toggle button, pressed or not.
T.AbstractButton {
    id: control

    property url iconSource
    property int iconSize: Theme.icon.md
    property color ink: Theme.text
    property color inkDisabled: Theme.textDisabled
    property color hoverFill: Theme.surface2
    property color restFill: Theme.library.clearPaint   // e.g. the always-on disc of the close button
    property bool active: false                     // toggled state (shuffle, repeat, lyrics)
    property bool toggle: false                     // `active` is an on/off state (accessibility)
    property color activeFill: Theme.accentSoft
    property color activeInk: ink
    property real fillRadius: height / 2            // hover disc
    property real activeRadius: fillRadius          // active state may use a rounded square
    property string toolTip: text

    implicitWidth: Math.max(Theme.size.iconButton, Theme.size.minimumHitTarget)
    implicitHeight: Math.max(Theme.size.iconButton, Theme.size.minimumHitTarget)
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    padding: 0

    Accessible.name: text
    Accessible.role: Accessible.Button
    Accessible.checkable: toggle || checkable
    Accessible.checked: toggle ? active : checked

    // Space activates the focused button (T.AbstractButton), ahead of any window Shortcut.
    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
    // Specific handlers on consumers (e.g. mute) get first refusal. A generic handler avoids
    // firing both an inherited Return signal handler and the consumer's own action.
    Keys.onPressed: event => {
        if (enabled && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) {
            clicked()
            event.accepted = true
        }
    }

    // Press: the button settles (scale, instant down / fast back). Hover: the hover fill
    // crossfades with the resting one (opacity: instant in, fast out). Transform and opacity only.
    readonly property bool hot: !active && enabled && (hovered || pressed)
    scale: pressed ? Theme.feel.pressScale : 1
    Behavior on scale {
        ScaleAnimator { duration: control.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
    }
    // C6: a short pop of the glyph (1 → likePop → 1, `base` in total), e.g. when a song is saved.
    function pop() { glyphPop.restart() }

    // Tooltip: built only while the pointer or the keyboard focus is on the button (none in
    // long lists at rest). T.ToolTip waits Theme.tip.delay
    // before showing; a press hides it. Opacity only, no timer of ours.
    Loader {
        active: control.visible && Theme.windowVisible && control.toolTip !== "" && control.enabled && !control.pressed
                && (control.hovered || control.visualFocus)
        sourceComponent: T.ToolTip {
            objectName: "iconToolTip"
            parent: control
            visible: true
            delay: Theme.effectsAllowed ? Theme.tip.delay : 0
            text: control.toolTip
            width: Math.min(Theme.tip.maxWidth, implicitWidth)
            x: Math.round((control.width - width) / 2)
            y: -height - Theme.tip.gap
            padding: Theme.tip.padding
            margins: Theme.space.xs          // kept inside the window; flips under the button near the top edge
            closePolicy: T.Popup.NoAutoClose
            enter: Transition { OpacityAnimator { from: 0; to: 1; duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut } }
            exit: null
            contentItem: LibraryLabel {
                text: control.toolTip
                wrapMode: Text.Wrap
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
            }
            background: Rectangle {
                color: Theme.surface2
                radius: Theme.radius.md
                RectangularShadow {
                    z: -1
                    anchors.fill: parent
                    radius: parent.radius
                    offset.y: Theme.elevation.popover.offsetY
                    blur: Theme.elevation.popover.blur
                    spread: Theme.elevation.popover.spread
                    color: Theme.elevation.popoverShadow
                }
            }
        }
    }

    background: Item {
        Rectangle {   // resting / active fill
            anchors.fill: parent
            radius: control.active ? control.activeRadius : control.fillRadius
            color: control.active ? control.activeFill : control.restFill
            opacity: control.hot ? 0 : 1
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, control.hot ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // hover fill
            anchors.fill: parent
            radius: control.fillRadius
            color: control.hoverFill
            opacity: control.hot ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, control.hot ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
        Rectangle {   // keyboard focus ring
            anchors.fill: parent
            visible: control.visualFocus
            radius: control.active ? control.activeRadius : control.fillRadius
            color: Theme.library.clearPaint
            border.width: Theme.stroke.focus
            border.color: control.ink
        }
    }

    contentItem: Item {
        IconImage {
            id: glyph
            anchors.centerIn: parent
            source: control.iconSource
            sourceSize: Qt.size(control.iconSize, control.iconSize)
            color: !control.enabled ? control.inkDisabled
                 : control.active ? control.activeInk
                 : control.ink
        }
        SequentialAnimation {
            id: glyphPop
            ScaleAnimator { target: glyph; from: 1; to: Theme.feel.likePop; duration: Theme.motion.instant; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            ScaleAnimator { target: glyph; to: 1; duration: Theme.motion.base - Theme.motion.instant; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        }
    }
}
