import QtQuick
import "../../theme"

Text {
    color: Theme.text
    font.family: Theme.family
    font.pixelSize: Theme.type.body
    font.weight: Theme.type.regular
    font.letterSpacing: Theme.type.tracking(Theme.type.body)
    elide: Text.ElideRight
}
