import QtQuick
import QtQuick.Templates as T
import QtQuick.Controls.impl
import "../../theme"

T.TextField {
    id: control
    property string accessibleName: ""
    implicitHeight: Theme.size.titleBar
    padding: Theme.space.md
    color: Theme.text
    placeholderTextColor: Theme.textSecondary
    selectionColor: Theme.accent
    selectedTextColor: Theme.accentInk
    font.family: Theme.family
    font.pixelSize: Theme.type.body
    selectByMouse: true
    hoverEnabled: true
    opacity: enabled ? 1 : Theme.library.disabledOpacity
    Accessible.role: Accessible.EditableText
    Accessible.name: accessibleName !== "" ? accessibleName : placeholderText
    // T.TextField draws no placeholder by itself (the Basic style adds this same item): without it
    // "¿Qué quieres escuchar?", "Buscar en la playlist"… never showed.
    PlaceholderText {
        x: control.leftPadding
        y: control.topPadding
        width: control.width - (control.leftPadding + control.rightPadding)
        height: control.height - (control.topPadding + control.bottomPadding)
        text: control.placeholderText
        font: control.font
        color: control.placeholderTextColor
        verticalAlignment: control.verticalAlignment
        elide: Text.ElideRight
        renderType: control.renderType
        visible: !control.length && !control.preeditText
    }
    background: Rectangle {
        color: Theme.surface2
        radius: Theme.radius.field
        // controlBorder: the field's edge stays findable (>= 3:1, WCAG 1.4.11).
        border.color: control.activeFocus ? Theme.focusRing : control.hovered ? Theme.textSecondary : Theme.controlBorder
        border.width: control.activeFocus ? Theme.stroke.focus : Theme.stroke.hairline
    }
}
