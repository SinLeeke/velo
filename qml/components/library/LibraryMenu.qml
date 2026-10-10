import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import QtQuick.Effects
import Velo.Backend as Backend
import "../../theme"
import "../../nav"
import "../motion"

Menu {
    id: libraryMenu
    // Reusable for track, artist, album and playlist menus when a seed URI is supplied.
    property string seedUri: ""
    // Q1: show disabled actions dimmed instead of hiding them (the queue menu explains why).
    property bool showDisabled: false
    property Action radioAction: Action {
        text: qsTr("Iniciar radio")
        enabled: libraryMenu.seedUri !== ""
        onTriggered: Backend.Player.startRadio(libraryMenu.seedUri)
    }
    property Action saveRadioAction: Action {
        text: qsTr("Guardar radio como playlist")
        enabled: libraryMenu.seedUri !== ""
        onTriggered: Backend.Library.saveRadioAsPlaylist(libraryMenu.seedUri)
    }
    // M1: "Compartir → Copiar enlace" (open.spotify.com link of the seed URI). R4: the link is
    // asked from Rust when the menu opens, never from a binding of every menu that exists.
    property string shareLink: ""
    property Action copyLinkAction: Action {
        text: qsTr("Copiar enlace")
        enabled: libraryMenu.shareLink !== ""
        onTriggered: Nav.copy(libraryMenu.shareLink)
    }
    onAboutToShow: shareLink = seedActions && seedUri !== "" ? Backend.Actions.shareUrl(seedUri) : ""
    // Submenus (e.g. the sleep timer) reuse the look without the seed actions.
    property bool seedActions: true
    Component.onCompleted: {
        if (!seedActions)
            return
        // Append after the existing actions, preserving their order and keyboard behavior.
        addAction(radioAction)
        addAction(saveRadioAction)
        addAction(copyLinkAction)
    }
    // R4: entries that carry an icon (e.g. the check of the active choice). While any entry has
    // one, every label starts after the icon column, so the labels stay aligned.
    property int iconEntries: 0
    implicitWidth: Theme.size.sidebar + Theme.space.xxl
    padding: Theme.space.xs
    font.family: Theme.family
    font.pixelSize: Theme.type.body
    // Motion (S1-2): opened tens of times a day (also with Shift+F10), so almost imperceptible:
    // fade + scale 0.98 -> 1 from its top edge (where the pointer or the "…" button is), `instant`.
    // Closing is immediate. Action state changes stay instant.
    transformOrigin: Popup.Top
    enter: null
    property PopupEntrance entrance: PopupEntrance { popup: libraryMenu; fromScale: Theme.entrance.menuScale; duration: Theme.motion.instant }
    exit: null
    delegate: MenuItem {
        id: entry
        implicitHeight: Theme.size.titleBar
        visible: enabled || libraryMenu.showDisabled
        height: visible ? implicitHeight : 0
        font: libraryMenu.font
        // U5: optional leading icon (Action { icon.source: … }), tinted like the label. Menus
        // whose actions have no icon keep the plain label, at the same position.
        readonly property bool hasIcon: entry.icon.source.toString() !== ""
        property bool counted: false
        function countIcon() {
            if (hasIcon === counted) return
            libraryMenu.iconEntries += hasIcon ? 1 : -1
            counted = hasIcon
        }
        onHasIconChanged: countIcon()
        Component.onCompleted: countIcon()
        Component.onDestruction: if (counted) libraryMenu.iconEntries--
        readonly property bool iconColumn: libraryMenu.iconEntries > 0
        contentItem: Row {
            spacing: Theme.space.md
            Item {
                visible: entry.iconColumn
                width: Theme.icon.md
                height: entry.availableHeight
                IconImage {
                    visible: entry.hasIcon
                    anchors.verticalCenter: parent.verticalCenter
                    source: entry.icon.source
                    sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                    color: entry.enabled ? Theme.textSecondary : Theme.textTertiary
                }
            }
            LibraryLabel {
                width: entry.availableWidth - (entry.iconColumn ? Theme.icon.md + parent.spacing : 0)
                height: entry.availableHeight
                text: entry.text
                color: entry.enabled ? Theme.text : Theme.textTertiary
                verticalAlignment: Text.AlignVCenter
            }
        }
        background: Rectangle {
            radius: Theme.radius.sm
            color: entry.highlighted ? Theme.surface3 : Theme.surface2
        }
    }
    // P2-1: lifted off the rows and cards (also surface2 on hover) by the popover elevation.
    // One RectangularShadow (SDF, single draw call, no layer).
    background: Rectangle {
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
            color: Theme.library.contextMenuShadow
        }
    }
}
