import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import QtQuick.Effects
import Velo.Backend as Backend
import "../../theme"
import "../motion"

// F4: "Clasificar por" and "Ver como" for the open playlist / "Tus me gusta". Choosing the
// current key again flips the direction (Rust decides; this menu only reflects the state).
Menu {
    id: menu
    // Motion (S1-2): like LibraryMenu, a near-imperceptible fade + scale 0.98 -> 1 from the top
    // edge (the sort button); closing is immediate.
    transformOrigin: Popup.Top
    enter: null
    property PopupEntrance entrance: PopupEntrance { popup: menu; fromScale: Theme.entrance.menuScale; duration: Theme.motion.instant }
    exit: null
    implicitWidth: Theme.trackList.menuWidth
    padding: Theme.space.xs
    font.family: Theme.family
    font.pixelSize: Theme.type.body
    background: PopoverBackground {}

    function labelFor(key) {
        switch (key) {
        case "title": return qsTr("Título")
        case "artist": return qsTr("Artista")
        case "album": return qsTr("Álbum")
        case "added": return qsTr("Agregado recientemente")
        case "duration": return qsTr("Duración")
        default: return qsTr("Orden personalizado")
        }
    }

    // R3: the inline components below are self-contained (no ids of this file), so other menus
    // reuse them as SortMenu.Choice / SortMenu.SectionLabel / SortMenu.PopoverBackground (the
    // player bar's visualizer menu, the sidebar's «Ordenar por»). Menu sizes its items to its own
    // width, so they need no width binding.
    //
    // Popover surface with elevation (P2-1): one RectangularShadow under the plate, no layer.
    component PopoverBackground: Rectangle {
        color: Theme.surface2
        radius: Theme.radius.md
        border.width: Theme.stroke.hairline
        border.color: Theme.hairline
        RectangularShadow {
            z: -1
            anchors.fill: parent
            radius: parent.radius
            offset.y: Theme.elevation.popover.offsetY
            blur: Theme.elevation.popover.blur
            spread: Theme.elevation.popover.spread
            color: Theme.library.popoverShadow
        }
    }

    component SectionLabel: LibraryLabel {
        leftPadding: Theme.space.md
        topPadding: Theme.space.sm
        bottomPadding: Theme.space.xs
        color: Theme.textSecondary
        font.pixelSize: Theme.type.caption
        font.weight: Theme.type.semibold
    }

    component Choice: MenuItem {
        id: choice
        property bool selected: false
        // Sort choices show the direction; view choices a check.
        property bool directional: false
        implicitHeight: Theme.size.titleBar
        font.family: Theme.family
        font.pixelSize: Theme.type.body
        Accessible.role: Accessible.RadioButton
        Accessible.checkable: true
        Accessible.checked: selected
        Accessible.name: directional && selected
                         ? (Backend.Library.sortDescending ? qsTr("%1, descendente").arg(text) : qsTr("%1, ascendente").arg(text))
                         : text
        contentItem: Item {
            implicitHeight: Theme.size.titleBar
            LibraryLabel {
                anchors.left: parent.left
                anchors.right: mark.left
                anchors.rightMargin: Theme.space.sm
                anchors.verticalCenter: parent.verticalCenter
                text: choice.text
                color: Theme.text
                font.weight: choice.selected ? Theme.type.semibold : Theme.type.regular
            }
            IconImage {
                id: mark
                visible: choice.selected
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                source: choice.directional ? Qt.resolvedUrl("../../icons/expand_more.svg") : Qt.resolvedUrl("../../icons/check.svg")
                sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                // Ascending points up (the chevron turned over), descending down.
                rotation: choice.directional && !Backend.Library.sortDescending ? 180 : 0
                color: Theme.accent
            }
        }
        background: Rectangle {
            radius: Theme.radius.sm
            color: choice.highlighted ? Theme.surface3 : Theme.surface2
        }
    }

    SectionLabel { text: qsTr("Clasificar por") }
    Choice { objectName: "sortCustom"; text: menu.labelFor("custom"); directional: true; selected: Backend.Library.sortKey === "custom"; onTriggered: Backend.Library.setSort("custom") }
    Choice { objectName: "sortTitle"; text: menu.labelFor("title"); directional: true; selected: Backend.Library.sortKey === "title"; onTriggered: Backend.Library.setSort("title") }
    Choice { objectName: "sortArtist"; text: menu.labelFor("artist"); directional: true; selected: Backend.Library.sortKey === "artist"; onTriggered: Backend.Library.setSort("artist") }
    Choice { objectName: "sortAlbum"; text: menu.labelFor("album"); directional: true; selected: Backend.Library.sortKey === "album"; onTriggered: Backend.Library.setSort("album") }
    Choice { objectName: "sortAdded"; text: menu.labelFor("added"); directional: true; selected: Backend.Library.sortKey === "added"; onTriggered: Backend.Library.setSort("added") }
    Choice { objectName: "sortDuration"; text: menu.labelFor("duration"); directional: true; selected: Backend.Library.sortKey === "duration"; onTriggered: Backend.Library.setSort("duration") }
    MenuSeparator {
        contentItem: Rectangle {
            implicitWidth: menu.availableWidth
            implicitHeight: Theme.stroke.hairline
            color: Theme.hairline
        }
    }
    SectionLabel { text: qsTr("Ver como") }
    Choice { objectName: "viewCompact"; text: qsTr("Compacta"); selected: Backend.Library.compactView; onTriggered: Backend.Library.setCompact(true) }
    Choice { objectName: "viewList"; text: qsTr("Lista"); selected: !Backend.Library.compactView; onTriggered: Backend.Library.setCompact(false) }
}
