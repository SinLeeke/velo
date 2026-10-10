import QtQuick
import QtQuick.Templates as T
import "../../theme"

T.ScrollBar {
    id: bar
    implicitWidth: Theme.space.sm
    minimumSize: Theme.library.scrollMinimumFraction
    padding: Theme.space.xxs
    policy: T.ScrollBar.AsNeeded
    contentItem: Rectangle {
        implicitWidth: Theme.space.xs
        radius: width / 2
        color: bar.pressed ? Theme.accent : Theme.textTertiary
        visible: bar.size < 1
    }
}
