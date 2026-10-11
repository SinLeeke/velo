import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import QtQuick.Controls.impl
import Velo.Backend
import "../../theme"
import "../controls"
import "../motion"

// Spotify Connect device popover. Public API (from T.Popup): open(), close(), opened;
// position it with x/y relative to its parent (e.g. under the devices button).
// Shows Player.devices (icon by `kind`, the active one marked), transfers on click/Enter and
// refreshes the list every time it opens. Up/Down move, Enter transfers, Esc closes.
// Motion (S1-2): fades in + scales 0.97 -> 1 from `originCorner` (bottom-right by default, where
// the devices button of the player bar is; Now Playing opens it under its tool pill); leaves shorter.
// R2: while a refresh with no devices yet is on its way, static skeleton rows instead of the empty
// state; devicesRefreshed ends the wait, including empty results and errors. No UI timer.
T.Popup {
    id: root

    readonly property int maxRows: Theme.nowPlaying.deviceMaxRows
    property int originCorner: T.Popup.BottomRight

    // Material Symbols icon for a Spotify Connect device type.
    function iconFor(kind) {
        const k = (kind || "").toLowerCase()
        if (k === "computer")
            return "computer"
        if (k === "smartphone" || k === "tablet")
            return "smartphone"
        if (k === "speaker" || k === "castaudio" || k === "avr" || k === "audiodongle")
            return "speaker"
        if (k === "tv" || k === "castvideo" || k === "stb" || k === "gameconsole")
            return "tv"
        return "devices"
    }

    width: Theme.panel.devicePopoverWidth
    height: contentItem.implicitHeight + topPadding + bottomPadding   // a bare Popup computes 0 for a Column content
    padding: Theme.space.sm
    modal: false
    focus: true
    closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
    transformOrigin: originCorner
    enter: null
    property PopupEntrance entrance: PopupEntrance { popup: root }
    exit: PopupExitTransition { effectsOn: entrance.effectsOn }

    function refresh() {
        refreshing = true
        errorCode = ""
        Player.refreshDevices()
    }
    onAboutToShow: refresh()
    onClosed: { refreshing = false; pendingId = "" }

    // E-1: a transfer is not instant (Spotify has to answer). The picked row shows «Conectando…»
    // (its subtitle fades in, `fast`) and the popup stays until that device is the active one.
    // It gives up (back to the plain row, still open) when Player reports an error or after
    // `connectGiveUp` ms. PauseAnimation, not a Timer: it only exists while a transfer is pending.
    property string pendingId: ""
    readonly property int connectGiveUp: Theme.motion.connectGiveUp
    onPendingIdChanged: if (pendingId !== "") giveUp.restart(); else giveUp.stop()
    PauseAnimation { id: giveUp; duration: root.connectGiveUp; onFinished: root.pendingId = "" }

    // Waiting for the refresh asked on opening (only matters while the list is empty).
    property bool refreshing: false
    property string errorCode: ""
    readonly property bool loading: refreshing && list.count === 0
    Connections {
        target: Player
        function onError(message) { root.pendingId = "" }
        function onDevicesRefreshed(errorCode) {
            root.errorCode = errorCode
            root.refreshing = false
            if (root.opened) {
                if (errorCode !== "" || list.count === 0) retry.forceActiveFocus()
                else list.forceActiveFocus()
            }
        }
    }
    function statusText() {
        if (errorCode === "auth") return qsTr("Inicia sesión de nuevo en Spotify para ver tus dispositivos")
        if (errorCode === "rateLimited") return qsTr("Espera un momento y vuelve a buscar tus dispositivos")
        if (errorCode !== "") return qsTr("No se pudieron cargar tus dispositivos. Comprueba la conexión y reintenta")
        return qsTr("No se encontraron dispositivos. Abre Spotify en tu celular o activa la reproducción en Velo")
    }
    onOpened: {
        let active = 0
        for (let i = 0; i < Player.devices.count; ++i)
            if (Player.devices.get(i).isActive)
                active = i
        list.currentIndex = active
        list.keyboardNav = false
        if (!refreshing && (errorCode !== "" || list.count === 0)) retry.forceActiveFocus()
        else list.forceActiveFocus()
    }

    background: Rectangle {
        radius: Theme.radius.lg
        color: Theme.surface1
        border.width: Theme.stroke.hairline
        border.color: Theme.hairline

        RectangularShadow {
            z: -1   // behind the card
            anchors.fill: parent
            radius: parent.radius
            offset.y: Theme.elevation.popover.offsetY
            blur: Theme.elevation.popover.blur
            spread: Theme.elevation.popover.spread
            color: Qt.rgba(0, 0, 0, Theme.elevation.popover.alpha)
        }
    }

    contentItem: Column {
        spacing: 0

        Text {
            x: Theme.space.md
            height: implicitHeight + 2 * Theme.space.sm
            verticalAlignment: Text.AlignVCenter
            text: qsTr("Dispositivos")
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.weight: Theme.type.semibold
            color: Theme.textSecondary
            Accessible.role: Accessible.Heading
        }

        ListView {
            id: list
            width: parent.width
            height: Math.min(contentHeight, root.maxRows * Theme.panel.deviceRow)
            visible: count > 0
            clip: true
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds
            model: Player.devices
            keyNavigationEnabled: true
            property bool keyboardNav: false   // focus ring only once the keyboard is used
            Keys.onPressed: keyboardNav = true
            currentIndex: -1

            Accessible.role: Accessible.List
            Accessible.name: qsTr("Dispositivos")

            Keys.onReturnPressed: if (currentItem) currentItem.activate()
            Keys.onEnterPressed: if (currentItem) currentItem.activate()
            Keys.onSpacePressed: if (currentItem) currentItem.activate()
            // Space activates a device here (the window's play/pause shortcut must not take it).
            Keys.onShortcutOverride: event => event.accepted = (event.key === Qt.Key_Space)

            delegate: T.AbstractButton {
                id: dev
                required property var model
                required property int index
                readonly property bool active: model.isActive
                readonly property bool pending: root.pendingId !== "" && root.pendingId === model.deviceId && !active
                onActiveChanged: if (active && root.pendingId === model.deviceId) root.close()

                width: ListView.view.width
                height: Theme.panel.deviceRow
                hoverEnabled: true
                focusPolicy: Qt.NoFocus
                padding: 0
                text: model.name

                Accessible.name: active ? qsTr("%1, dispositivo activo").arg(text) : text
                Accessible.role: Accessible.Button

                function activate() {
                    if (!model.deviceId || root.pendingId !== "") return
                    if (active) { root.close(); return }
                    root.pendingId = model.deviceId
                    Player.transferTo(model.deviceId)
                    if (model.isActive) root.close()   // the transfer already finished (local engine)
                }
                onClicked: activate()

                background: Rectangle {
                    radius: Theme.radius.md
                    color: dev.active ? Theme.accentSoft : dev.hovered || dev.pressed || dev.pending ? Theme.surface2 : Theme.library.clearPaint
                    border.width: dev.ListView.isCurrentItem && list.keyboardNav ? Theme.stroke.focus : 0
                    border.color: Theme.focusRing
                }
                contentItem: Item {
                    IconImage {
                        id: kindIcon
                        anchors.left: parent.left
                        anchors.leftMargin: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        source: Qt.resolvedUrl("../../icons/" + root.iconFor(dev.model.kind) + ".svg")
                        sourceSize: Qt.size(Theme.icon.lg, Theme.icon.lg)
                        color: dev.active ? Theme.accent : Theme.textSecondary
                    }
                    Column {
                        anchors.left: kindIcon.right
                        anchors.leftMargin: Theme.space.md
                        anchors.right: checkIcon.left
                        anchors.rightMargin: Theme.space.sm
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Theme.space.xxs
                        Text {
                            width: parent.width
                            text: dev.text
                            elide: Text.ElideRight
                            font.family: Theme.family
                            font.pixelSize: Theme.type.body
                            font.weight: Theme.type.semibold
                            color: dev.active ? Theme.accent : Theme.text
                        }
                        Text {
                            // The subtitle fades in for «Conectando…» and for «Reproduciendo aquí»
                            // (opacity, `fast`, easeOut); the picked row only, never the list.
                            visible: opacity > 0
                            opacity: dev.active || dev.pending ? 1 : 0
                            Behavior on opacity {
                                OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                            }
                            width: parent.width
                            text: dev.pending ? qsTr("Conectando…") : qsTr("Reproduciendo aquí")
                            elide: Text.ElideRight
                            font.family: Theme.family
                            font.pixelSize: Theme.type.caption
                            color: Theme.textSecondary
                        }
                    }
                    IconImage {
                        id: checkIcon
                        anchors.right: parent.right
                        anchors.rightMargin: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        visible: dev.active
                        source: Qt.resolvedUrl("../../icons/check.svg")
                        sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                        color: Theme.accent
                    }
                }
            }
        }

        // R2: refreshing with nothing to show yet: static skeleton rows (no shimmer, no timer).
        Column {
            objectName: "devicesSkeleton"
            visible: root.loading
            width: parent.width
            Repeater {
                model: root.loading ? Theme.nowPlaying.deviceSkeletonRows : 0
                delegate: Item {
                    width: parent ? parent.width : 0
                    height: Theme.panel.deviceRow
                    Rectangle {
                        id: skelIcon
                        x: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.icon.lg
                        height: width
                        radius: Theme.radius.sm
                        color: Theme.surface2
                    }
                    Rectangle {
                        anchors.left: skelIcon.right
                        anchors.leftMargin: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.panel.queueThumb * 3
                        height: Theme.space.sm
                        radius: height / 2
                        color: Theme.surface2
                    }
                }
            }
        }

        // Empty/error state; cached rows remain usable if a refresh fails.
        Item {
            objectName: "devicesStatus"
            visible: !root.refreshing && (list.count === 0 || root.errorCode !== "")
            width: parent.width
            height: visible ? emptyColumn.implicitHeight + 2 * Theme.space.xl : 0
            Column {
                id: emptyColumn
                anchors.centerIn: parent
                width: parent.width - 2 * Theme.space.lg
                spacing: Theme.space.md
                IconImage {
                    anchors.horizontalCenter: parent.horizontalCenter
                    source: Qt.resolvedUrl("../../icons/devices.svg")
                    sourceSize: Qt.size(Theme.icon.lg * 2, Theme.icon.lg * 2)
                    color: Theme.textTertiary
                }
                Text {
                    objectName: "devicesStatusText"
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    text: root.statusText()
                    font.family: Theme.family
                    font.pixelSize: Theme.type.body
                    font.weight: Theme.type.medium
                    lineHeight: Theme.type.bodyLineHeight
                    color: Theme.text
                }
                T.AbstractButton {
                    id: retry
                    objectName: "devicesRetry"
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: root.errorCode !== "" ? qsTr("Reintentar") : qsTr("Buscar de nuevo")
                    implicitWidth: retryText.implicitWidth + 2 * Theme.space.lg
                    implicitHeight: Theme.size.iconButton + Theme.space.sm
                    focusPolicy: Qt.StrongFocus
                    hoverEnabled: true
                    Accessible.role: Accessible.Button
                    Accessible.name: text
                    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
                    Keys.onReturnPressed: clicked()
                    Keys.onEnterPressed: clicked()
                    onClicked: root.refresh()
                    background: Rectangle {
                        radius: Theme.radius.md
                        color: retry.down || retry.hovered ? Theme.surface3 : Theme.surface2
                        border.width: retry.visualFocus ? Theme.stroke.focus : Theme.stroke.hairline
                        border.color: retry.visualFocus ? Theme.focusRing : Theme.hairline
                    }
                    contentItem: Text {
                        id: retryText
                        text: retry.text
                        color: Theme.text
                        font.family: Theme.family
                        font.pixelSize: Theme.type.body
                        font.weight: Theme.type.semibold
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }
        }
    }
}
