import QtQuick
import "../../theme"

// Segmented choice (ref_02: "Normal | Circular"). One tab stop; Left/Right (mirrored in RTL),
// Home and End change the choice.
// model: [{ value: "bars", label: qsTr("Barras") }, ...]; `current` is the selected value.
// Emits activated(value); the owner writes it to its setting (the control does not own state).
// Accessibility: the control is a group named `name`; each segment is a radio button with its
// checked state and a press action, so screen readers list and pick the options.
FocusScope {
    id: root

    property var model: []
    property string current: ""
    property string name: ""            // accessible name (qsTr)
    signal activated(string value)

    readonly property int currentIndex: {
        for (let i = 0; i < model.length; ++i)
            if (model[i].value === current)
                return i
        return -1
    }
    readonly property string currentLabel: currentIndex >= 0 ? model[currentIndex].label : ""
    property bool mouseFocus: false

    implicitWidth: track.implicitWidth
    implicitHeight: Theme.size.iconButton
    activeFocusOnTab: true

    Accessible.name: name
    Accessible.role: Accessible.Grouping
    Accessible.description: currentLabel

    function pick(i) {
        if (i >= 0 && i < model.length && model[i].value !== current) {
            track.armed = true
            activated(model[i].value)
        }
    }
    // Left/Right follow the reading direction (Row mirrors under LayoutMirroring).
    readonly property int forward: LayoutMirroring.enabled ? -1 : 1

    Keys.onLeftPressed: pick(Math.max(0, Math.min(model.length - 1, currentIndex - forward)))
    Keys.onRightPressed: pick(Math.max(0, Math.min(model.length - 1, currentIndex + forward)))
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Home) { pick(0); event.accepted = true }
        else if (event.key === Qt.Key_End) { pick(model.length - 1); event.accepted = true }
    }
    onActiveFocusChanged: if (!activeFocus) mouseFocus = false

    Rectangle {
        id: track
        anchors.fill: parent
        implicitWidth: row.implicitWidth + 2 * Theme.space.xs
        radius: Theme.radius.segment
        color: Theme.dark ? Theme.surface2 : Theme.surface3
        border.width: root.activeFocus && !root.mouseFocus ? Theme.stroke.focus : 0
        border.color: Theme.focusRing

        // One pill for the selected segment, under the labels. It slides between segments
        // (XAnimator, base, easeInOut); its width takes the new segment's at once (no width
        // animation). Armed by a pick so the first layout does not slide in. Its controlBorder
        // edge keeps the choice readable (>= 3:1) without relying on the fill alone.
        property bool armed: false
        readonly property Item selectedSeg: repeater.count > 0 && root.currentIndex >= 0 ? repeater.itemAt(root.currentIndex) : null
        Rectangle {
            visible: track.selectedSeg !== null
            x: row.x + (track.selectedSeg ? track.selectedSeg.x : 0)
            y: row.y
            width: track.selectedSeg ? track.selectedSeg.width : 0
            height: row.height
            radius: Theme.radius.segmentInner
            color: Theme.dark ? Theme.surface3 : Theme.surface1
            border.width: Theme.stroke.hairline
            border.color: Theme.controlBorder
            Behavior on x {
                enabled: track.armed
                XAnimator { duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }
        }

        Row {
            id: row
            anchors.fill: parent
            anchors.margins: Theme.space.xs
            spacing: 0

            Repeater {
                id: repeater
                model: root.model
                delegate: Rectangle {
                    id: seg
                    required property var modelData
                    required property int index
                    readonly property bool selected: modelData.value === root.current

                    width: label.implicitWidth + 2 * Theme.space.lg
                    height: row.height
                    radius: Theme.radius.segmentInner
                    // The selected fill is the sliding pill above; segments only show hover.
                    color: !selected && mouse.containsMouse ? Theme.hairline : Theme.library.clearPaint

                    Accessible.role: Accessible.RadioButton
                    Accessible.name: modelData.label
                    Accessible.checkable: true
                    Accessible.checked: selected
                    Accessible.onPressAction: root.pick(index)

                    Text {
                        id: label
                        anchors.centerIn: parent
                        text: seg.modelData.label
                        font.family: Theme.family
                        font.pixelSize: Theme.type.body
                        font.weight: seg.selected ? Theme.type.semibold : Theme.type.medium
                        color: seg.selected ? Theme.text : Theme.textSecondary
                        Accessible.ignored: true
                    }

                    MouseArea {
                        id: mouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onPressed: { root.mouseFocus = true; root.forceActiveFocus() }
                        onClicked: root.pick(seg.index)
                    }
                }
            }
        }
    }
}
