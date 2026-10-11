import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Effects
import QtQuick.Controls.impl
import QtQuick.Templates as T
import QtCore as QtCore
import Velo.Backend
import "../../theme"
import "../"
import "../library"
import "../nowplaying"

// Queue panel, docked at the right edge between the 44 px title strip and the player bar.
// It is laid out, not floated: the container gives the view a matching `queueInset` so the
// page narrows instead of being covered (Main.qml). Public API: open(), close(), opened,
// bottomInset (space kept free at the bottom, the library player bar), and the requests
// addToPlaylistRequested(uri), artistRequested(id), albumRequested(id).
//
// "A continuación" rows (Player.queue): a click or Enter plays that song (R2, Player.skipToQueue;
// without the backend method, activation is disabled). They can be edited only with Velo as the playing
// device (Player.queueEditable): drag by the handle, right click or the "…" button for the menu,
// or the keyboard (Menu/Shift+F10 menu, Alt+Up/Down move, Delete remove). Otherwise the editing
// actions are disabled. One shared menu; ListView with reused delegates
// and 40 px async thumbnails. No timers. Motion (S1-2): it slides in from the right with a fade;
// the page narrows at once (only the panel's transform and opacity animate, never a width).
FocusScope {
    id: root
    property bool onCover: false     // shown over Now Playing: tinted like the room

    property bool opened: false
    property real bottomInset: 0
    readonly property bool effectsOn: Player.isPlaying && visible && Window.window !== null
        && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || Window.active)
    readonly property int fastDuration: effectsOn ? Theme.motion.fast : 0
    readonly property int baseDuration: effectsOn ? Theme.motion.base : 0
    readonly property int slowDuration: effectsOn ? Theme.motion.slow : 0
    readonly property int instantDuration: effectsOn ? Theme.motion.instant : 0

    signal addToPlaylistRequested(string uri)
    signal artistRequested(string artistId)
    signal albumRequested(string albumId)

    function open() {
        Player.refreshQueue()
        opened = true
        if (showRecents) { nowMs = Date.now(); if (History.error !== "") History.load() }
        closeButton.forceActiveFocus()
    }
    function close() {
        rowMenu.close()
        panelMenu.close()
        nameDialog.close()
        endDrag(false)
        opened = false
    }

    // Enters with opacity 0 -> 1 and a Translate of 15% of its width -> 0 (slow, ease-out); leaves
    // the same way back, shorter (base, ease-in). Visible while the exit plays: the Animator keeps
    // the QML opacity until it ends, so `opacity > 0` holds through the fade-out.
    visible: opened || opacity > 0
    opacity: opened ? 1 : 0
    Behavior on opacity {
        OpacityAnimator {
            duration: Math.max(1, root.opened ? root.slowDuration : root.baseDuration)
            easing.type: Easing.Bezier
            easing.bezierCurve: root.opened ? Theme.motion.easeOut : Theme.motion.easeIn
        }
    }
    // A Translate is not an Item, so no Animator can drive it: a NumberAnimation of its x only
    // (a transform, no relayout), and only while the panel opens or closes.
    transform: Translate {
        x: root.opened ? 0 : root.width * Theme.entrance.queueShift
        Behavior on x {
            NumberAnimation {
                duration: root.opened ? root.slowDuration : root.baseDuration
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.motion.easeOut : Theme.motion.easeIn
            }
        }
    }
    anchors.right: parent.right
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    anchors.topMargin: Theme.size.titleBar + Theme.queue.inset
    anchors.bottomMargin: bottomInset + Theme.queue.inset
    // G4: resizable by dragging its left edge (ResizeHandle below), between queueMinWidth and
    // queueMaxWidth (and `maxWidth`, which the container lowers so the page keeps room). The width
    // is remembered between sessions at zoom 100 % (QtCore.Settings, a local convenience), so it
    // scales with the interface zoom. While dragging it follows the pointer, never animates.
    property real maxWidth: Theme.panel.queueMaxWidth
    property real dragWidth: -1
    QtCore.Settings {
        id: layoutStore
        category: "layout"
        property real queueWidth: 0         // at zoom 1; 0 = default
    }
    readonly property real preferredWidth: dragWidth > 0 ? dragWidth
        : layoutStore.queueWidth > 0 ? Theme.px(layoutStore.queueWidth) : Theme.queue.width
    readonly property real widthCeiling: Math.max(Theme.panel.queueMinWidth, Math.min(Theme.panel.queueMaxWidth, maxWidth))
    width: Math.round(Math.max(Theme.panel.queueMinWidth, Math.min(widthCeiling, preferredWidth)))
    z: 90

    Keys.onEscapePressed: root.close()

    // Millisecond duration as m:ss.
    function fmt(ms) {
        const s = Math.max(0, Math.round(ms / 1000))
        return Math.floor(s / 60) + ":" + (s % 60 < 10 ? "0" : "") + (s % 60)
    }

    // ---------------------------------------------------------------- tabs (N4)
    // 0 = "Cola" (what follows, editable), 1 = "Recientes" (History: local, read-only).
    property int tab: 0
    readonly property bool showRecents: tab === 1
    property bool recentsLoaded: false
    // "Now" for the relative dates: taken when the tab opens and when a batch arrives, so the
    // texts need no Timer (they are as fresh as the moment the batch is shown).
    property double nowMs: Date.now()
    // The batch for which loadMore() was already asked (one request per batch).
    property int moreRequestedAt: -1

    function selectTab(next) {
        if (next === tab)
            return
        rowMenu.close()
        historyMenu.close()
        endDrag(false)
        tab = next
        if (next === 1) {
            nowMs = Date.now()
            if (!recentsLoaded) {
                recentsLoaded = true
                History.load()
            }
        }
    }
    // Relative date, computed when shown (Spanish/English through qsTr plurals).
    function ago(playedAtMs) {
        const minutes = Math.floor(Math.max(0, nowMs - playedAtMs) / 60000)
        if (minutes < 1)
            return qsTr("justo ahora")
        if (minutes < 60)
            return qsTr("hace %n minuto(s)", "", minutes)
        const hours = Math.floor(minutes / 60)
        if (hours < 24)
            return qsTr("hace %n hora(s)", "", hours)
        const days = Math.floor(hours / 24)
        if (days < 30)
            return qsTr("hace %n día(s)", "", days)
        const months = Math.floor(days / 30)
        if (months < 12)
            return qsTr("hace %n mes(es)", "", months)
        return qsTr("hace %n año(s)", "", Math.floor(days / 365))
    }
    // Near the end of the loaded rows: ask for the next batch once.
    function maybeLoadMore() {
        const n = recentsList.count
        if (!opened || !showRecents || !History.hasMore || History.loading || History.error !== "" || n === 0 || moreRequestedAt === n)
            return
        if (recentsList.contentY + recentsList.height >= recentsList.contentHeight - 4 * Theme.size.trackRow) {
            moreRequestedAt = n
            History.loadMore()
        }
    }
    Connections {
        target: History
        function onChanged() { root.nowMs = Date.now() }
        function onCleared() { root.moreRequestedAt = -1; root.nowMs = Date.now() }
        function onLoadingChanged() { if (!History.loading) root.maybeLoadMore() }
        function onErrorChanged() { if (History.error !== "") root.moreRequestedAt = -1 }
    }

    // ---------------------------------------------------------------- editing
    readonly property bool editable: Player.queueEditable
    // Drag in progress: the row picked up and the boundary it would land on (0..count).
    property int dragFrom: -1
    property int dropBoundary: -1
    // Pointer over a drag handle: the list stops flicking so the handle gets the drag.
    property bool handleHovered: false

    function moveRow(from, to) {
        if (!editable || from < 0 || to < 0 || from >= list.count || to >= list.count || from === to)
            return
        Player.moveInQueue(from, to)
    }
    function removeRow(row) {
        if (editable && row >= 0 && row < list.count)
            Player.removeFromQueue(row)
    }
    // R2: jump to a queued song (the rows before it are skipped). Needs the backend's
    // Player.skipToQueue; remote Connect cannot edit or jump within this queue.
    readonly property bool rowsPlayable: typeof Player.skipToQueue === "function" && Player.canControl && editable
    function playRow(row) {
        if (rowsPlayable && row >= 0 && row < list.count)
            Player.skipToQueue(row)
    }
    // The boundary under a point of the list's content (between rows, 0 = before the first).
    function boundaryAt(contentY) {
        const first = list.originY + (list.headerItem ? list.headerItem.height : 0)
        return Math.max(0, Math.min(list.count, Math.round((contentY - first) / Theme.size.trackRow)))
    }
    function updateDrag(item, pointY) {
        dropBoundary = boundaryAt(item.mapToItem(list.contentItem, 0, pointY).y)
    }
    function endDrag(commit) {
        const from = dragFrom, boundary = dropBoundary
        dragFrom = -1
        dropBoundary = -1
        if (commit && from >= 0 && boundary >= 0)
            moveRow(from, boundary > from ? boundary - 1 : boundary)
    }

    // One menu for every row. `row` is the delegate that opened it (closed if it gets reused).
    LibraryMenu {
        id: rowMenu
        objectName: "queueRowMenu"
        property Item row: null
        readonly property int index: row ? row.index : -1
        readonly property bool liked: row ? row.liked : false
        seedUri: row ? row.uri : ""
        showDisabled: true
        function openFor(item, atPointer) {
            row = item
            Player.inspectQueueTrack(item.index)   // the heart's real state for this row only
            if (atPointer)
                popup()
            else
                popup(item, item.width - width - Theme.space.sm, item.height)
        }
        onClosed: row = null

        Action {
            text: qsTr("Quitar de la cola")
            enabled: root.editable
            onTriggered: root.removeRow(rowMenu.index)
        }
        Action {
            text: qsTr("Subir")
            enabled: root.editable && rowMenu.index > 0
            onTriggered: root.moveRow(rowMenu.index, rowMenu.index - 1)
        }
        Action {
            text: qsTr("Bajar")
            enabled: root.editable && rowMenu.index >= 0 && rowMenu.index < list.count - 1
            onTriggered: root.moveRow(rowMenu.index, rowMenu.index + 1)
        }
        Action {
            text: qsTr("Mover al principio")
            enabled: root.editable && rowMenu.index > 0
            onTriggered: root.moveRow(rowMenu.index, 0)
        }
        Action {
            text: qsTr("Añadir a playlist…")
            enabled: rowMenu.seedUri !== ""
            onTriggered: root.addToPlaylistRequested(rowMenu.seedUri)
        }
        Action {
            text: rowMenu.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Añadir a Tus me gusta")
            enabled: rowMenu.index >= 0
            onTriggered: Player.setQueueLiked(rowMenu.index, !rowMenu.liked)
        }
        Action {
            text: qsTr("Ir al artista")
            enabled: rowMenu.row !== null && rowMenu.row.artistId !== ""
            onTriggered: root.artistRequested(rowMenu.row.artistId)
        }
        Action {
            text: qsTr("Ir al álbum")
            enabled: rowMenu.row !== null && rowMenu.row.albumId !== ""
            onTriggered: root.albumRequested(rowMenu.row.albumId)
        }
    }

    // N4: menu of a "Recientes" row. The song actions only (radio, link, playlist, artist, album):
    // History is a log, so nothing here edits the queue.
    readonly property alias recentsMenu: historyMenu
    LibraryMenu {
        id: historyMenu
        objectName: "queueHistoryMenu"
        property Item row: null
        seedUri: row ? row.uri : ""
        function openFor(item, atPointer) {
            row = item
            if (atPointer)
                popup()
            else
                popup(item, item.width - width - Theme.space.sm, item.height)
        }
        onClosed: row = null

        Action {
            text: qsTr("Añadir a playlist…")
            enabled: historyMenu.seedUri !== ""
            onTriggered: root.addToPlaylistRequested(historyMenu.seedUri)
        }
        Action {
            text: qsTr("Ir al artista")
            enabled: historyMenu.row !== null && historyMenu.row.artistId !== ""
            onTriggered: root.artistRequested(historyMenu.row.artistId)
        }
        Action {
            text: qsTr("Ir al álbum")
            enabled: historyMenu.row !== null && historyMenu.row.albumId !== ""
            onTriggered: root.albumRequested(historyMenu.row.albumId)
        }
    }

    // X2: panel actions. Clear needs Velo as the device; saving works with any device.
    function saveQueue() {
        const name = nameField.text.trim()
        if (name === "")
            return
        nameDialog.close()
        Player.saveQueueAsPlaylist(name)
    }
    // X2: name prompt for "Guardar como playlist", on the shared library dialog (M1b).
    readonly property alias saveDialog: nameDialog
    LibraryDialog {
        id: nameDialog
        objectName: "queueNameDialog"
        title: qsTr("Guardar cola como playlist")
        initialFocus: nameField
        LibraryField {
            id: nameField
            width: parent.width
            placeholderText: qsTr("Nombre de la playlist")
            Accessible.name: qsTr("Nombre de la playlist")
            onAccepted: root.saveQueue()
        }
        buttons: [
            LibraryButton {
                text: qsTr("Cancelar")
                onClicked: nameDialog.close()
            },
            LibraryButton {
                primary: true
                text: qsTr("Guardar")
                enabled: nameField.text.trim() !== ""
                onClicked: root.saveQueue()
            }
        ]
    }
    LibraryMenu {
        id: panelMenu
        objectName: "queuePanelMenu"
        seedActions: false
        showDisabled: true
        Action {
            text: qsTr("Vaciar cola")
            enabled: Player.queueClearable
            onTriggered: Player.clearQueue()
        }
        Action {
            text: qsTr("Guardar como playlist…")
            enabled: Player.hasSession && Player.title !== ""
            onTriggered: {
                nameField.text = ""
                nameDialog.open()
            }
        }
    }

    // One queue/now-playing row: [drag handle] thumbnail, title + artist, duration / "…".
    component QueueRow: FocusScope {
        id: row
        property int index: -1
        property string uri
        property string title
        property string artist
        property url imageUrl
        property int durationMs: 0
        property bool liked: false
        property string artistId
        property string albumId
        property bool current: false          // "Sonando ahora": no handle, no menu
        readonly property bool editing: !current && root.editable
        readonly property bool dragged: !current && root.dragFrom === index
        width: ListView.view ? ListView.view.width : parent.width
        height: Theme.size.trackRow
        activeFocusOnTab: false
        onActiveFocusChanged: if (activeFocus && !current) {
            list.currentIndex = index
            list.positionViewAtIndex(index, ListView.Contain)
        }

        // C-1: on a song change the "Sonando ahora" row (the only `current` one) fades its cover
        // and text back in over `fast` (opacity only, nothing moves), in step with the list row,
        // the side panel and the bar. Rows of the queue below are never touched.
        function swapIn() {
            if (!current || !visible || root.fastDuration <= 0)
                return
            swapAnim.restart()
        }
        onTitleChanged: swapIn()
        onImageUrlChanged: swapIn()
        ParallelAnimation {
            id: swapAnim
            OpacityAnimator { target: thumb; from: 0; to: 1; duration: Math.max(1, root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            OpacityAnimator { target: rowText; from: 0; to: 1; duration: Math.max(1, root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }

        Accessible.role: Accessible.ListItem
        Accessible.name: qsTr("%1, %2").arg(title).arg(artist)
        Accessible.onPressAction: activate()

        function openMenu(atPointer) {
            if (!current) {
                forceActiveFocus()
                rowMenu.openFor(row, atPointer)
            }
        }
        // Enter / click: play this song. The menu has its own desktop shortcuts.
        function activate() {
            if (current)
                return
            if (root.rowsPlayable)
                root.playRow(index)
        }
        Keys.onMenuPressed: openMenu(false)
        Keys.onReturnPressed: activate()
        Keys.onEnterPressed: activate()
        Keys.onSpacePressed: activate()
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onDeletePressed: root.removeRow(index)
        Keys.onPressed: event => {
            if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
                openMenu(false); event.accepted = true
            } else if (event.modifiers & Qt.AltModifier && event.key === Qt.Key_Up) {
                root.moveRow(index, index - 1); event.accepted = true
            } else if (event.modifiers & Qt.AltModifier && event.key === Qt.Key_Down) {
                root.moveRow(index, index + 1); event.accepted = true
            }
        }

        // C3: the row surface, the drag handle and the "…" fade with the hover (opacity only:
        // in at `instant`, out at `fast`), the same as the library's song rows.
        readonly property bool surfaceShown: dragged || activeFocus || rowHover.hovered
        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: Theme.space.sm
            anchors.rightMargin: Theme.space.sm
            radius: Theme.radius.md
            color: row.dragged ? Theme.queue.dragFill
                 : row.activeFocus ? Theme.accentSoft : Theme.surface2
            opacity: row.surfaceShown ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, row.surfaceShown ? root.instantDuration : root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            border.width: row.activeFocus ? Theme.stroke.focus : 0
            border.color: Theme.focusRing
        }
        HoverHandler { id: rowHover; cursorShape: !row.current && root.rowsPlayable ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler {
            enabled: !row.current && root.rowsPlayable
            acceptedButtons: Qt.LeftButton
            // A press on the drag handle belongs to the drag, not to "play".
            onTapped: (point) => {
                if (row.editing && point.position.x < handle.x + handle.width)
                    return
                row.forceActiveFocus(Qt.MouseFocusReason)
                root.playRow(row.index)
            }
        }
        TapHandler {
            enabled: !row.current
            acceptedButtons: Qt.RightButton
            onTapped: row.openMenu(true)
        }

        // Drag handle: only where editing works, so it never promises what cannot happen.
        Item {
            id: handle
            anchors.left: parent.left
            anchors.leftMargin: Theme.space.sm
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: Theme.queue.handleWidth
            visible: !row.current
            IconImage {
                id: grip
                anchors.centerIn: parent
                visible: row.editing
                readonly property bool shown: rowHover.hovered || row.activeFocus || row.dragged
                opacity: shown ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, grip.shown ? root.instantDuration : root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                source: Qt.resolvedUrl("../../icons/drag_indicator.svg")
                sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                color: row.dragged ? Theme.accent : Theme.textTertiary
            }
            HoverHandler {
                enabled: row.editing
                cursorShape: Qt.OpenHandCursor
                onHoveredChanged: root.handleHovered = hovered
            }
            DragHandler {
                id: drag
                enabled: row.editing
                target: null
                grabPermissions: PointerHandler.CanTakeOverFromAnything
                xAxis.enabled: false
                cursorShape: Qt.ClosedHandCursor
                onActiveChanged: {
                    if (active) {
                        root.dragFrom = row.index
                        root.updateDrag(handle, centroid.position.y)
                    } else {
                        root.endDrag(true)
                    }
                }
                onCentroidChanged: if (active) root.updateDrag(handle, centroid.position.y)
            }
        }

        Rectangle {
            id: thumb
            anchors.left: row.current ? parent.left : handle.right
            anchors.leftMargin: row.current ? Theme.space.lg : 0
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.panel.queueThumb
            height: width
            radius: Theme.radius.sm
            color: Theme.surface3
            IconImage {
                anchors.centerIn: parent
                visible: art.status !== Image.Ready
                source: Qt.resolvedUrl("../../icons/music_note.svg")
                sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                color: Theme.textTertiary
            }
            Image {
                id: art
                anchors.fill: parent
                source: row.imageUrl
                asynchronous: true
                sourceSize: Qt.size(Theme.panel.queueThumb * 2, Theme.panel.queueThumb * 2)   // 2x for HiDPI, still tiny
                fillMode: Image.PreserveAspectCrop
                visible: status === Image.Ready
            }
        }
        Column {
            id: rowText
            anchors.left: thumb.right
            anchors.leftMargin: Theme.space.md
            anchors.right: trailing.left
            anchors.rightMargin: Theme.space.sm
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.xxs
            Text {
                width: parent.width
                text: row.title
                elide: Text.ElideRight
                font.family: Theme.family
                font.pixelSize: Theme.type.body
                font.weight: Theme.type.semibold
                color: row.current ? Theme.accent : Theme.text
            }
            Text {
                width: parent.width
                text: row.artist
                elide: Text.ElideRight
                font.family: Theme.family
                font.pixelSize: Theme.type.caption
                color: Theme.textSecondary
            }
        }
        // Duration at rest; the "…" button on hover/focus (same slot, no relayout).
        Item {
            id: trailing
            anchors.right: parent.right
            anchors.rightMargin: Theme.space.md
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.size.iconButton + Theme.space.xs
            height: Theme.size.iconButton
            readonly property bool showMore: !row.current && (rowHover.hovered || row.activeFocus || more.activeFocus)
            // Duration and "…" crossfade. NumberAnimation (not an Animator) so `visible` follows
            // the running opacity: the hidden one leaves the tab order once its fade ends.
            Text {
                anchors.right: parent.right
                anchors.rightMargin: Theme.space.xs
                anchors.verticalCenter: parent.verticalCenter
                opacity: trailing.showMore ? 0 : 1
                visible: opacity > 0
                Behavior on opacity {
                    NumberAnimation { duration: trailing.showMore ? root.instantDuration : root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                text: root.fmt(row.durationMs)
                font.family: Theme.family
                font.pixelSize: Theme.type.micro
                font.features: Theme.tabularFeatures
                color: Theme.textTertiary
            }
            IconButton {
                id: more
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                opacity: trailing.showMore ? 1 : 0
                visible: opacity > 0
                Behavior on opacity {
                    NumberAnimation { duration: trailing.showMore ? root.instantDuration : root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                text: qsTr("Opciones de %1").arg(row.title)
                iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                iconSize: Theme.icon.md
                ink: Theme.textSecondary
                onClicked: row.openMenu(false)
            }
        }
    }

    // N4: tab pill. A transparent button; the selected fill is the sliding `tabIndicator`
    // (behind the pills), so only the ink changes color here.
    component TabPill: T.Button {
        id: pill
        property bool selected: false
        implicitHeight: Theme.artistPage.chipHeight
        implicitWidth: root.pillWidth
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        Accessible.role: Accessible.PageTab
        Accessible.name: text
        Accessible.selected: selected
        // Space presses the focused pill (the window's play/pause shortcut must not take it).
        Keys.onShortcutOverride: event => event.accepted = (event.key === Qt.Key_Space)
        scale: down ? Theme.feel.pressScale : 1
        Behavior on scale {
            ScaleAnimator { duration: pill.down ? root.instantDuration : root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        background: Item {
            Rectangle {   // hover fill (idle pills only)
                anchors.fill: parent
                radius: height / 2
                color: Theme.surface3
                opacity: pill.hovered && !pill.selected ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, pill.hovered ? root.instantDuration : root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }
            Rectangle {   // keyboard focus ring
                anchors.fill: parent
                radius: height / 2
                visible: pill.visualFocus
                color: Theme.library.clearPaint
                border.width: Theme.stroke.focus
                border.color: Theme.focusRing
            }
        }
        // Selected ink and idle ink are two labels that crossfade (opacity only, no ColorAnimation).
        contentItem: Item {
            implicitWidth: idleInk.implicitWidth
            implicitHeight: idleInk.implicitHeight
            LibraryLabel {
                id: idleInk
                anchors.fill: parent
                text: pill.text
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                color: Theme.textSecondary
                opacity: pill.selected ? 0 : 1
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
            }
            LibraryLabel {
                anchors.fill: parent
                text: pill.text
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                color: Theme.accent
                opacity: pill.selected ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
            }
        }
    }
    // Both pills share the wider label's width, so the indicator only has to slide (x).
    readonly property real pillWidth: Math.max(tabMetricsA.advanceWidth, tabMetricsB.advanceWidth) + 2 * Theme.space.lg
    TextMetrics { id: tabMetricsA; text: qsTr("Cola"); font.family: Theme.family; font.pixelSize: Theme.type.caption; font.weight: Theme.type.semibold }
    TextMetrics { id: tabMetricsB; text: qsTr("Recientes"); font.family: Theme.family; font.pixelSize: Theme.type.caption; font.weight: Theme.type.semibold }

    // One "Recientes" row: thumbnail, title (accent if it is playing), "artist • ago", duration.
    // Click plays that entry; the menu has the song actions only.
    component RecentRow: FocusScope {
        id: rrow
        property string entryId
        property string uri
        property string title
        property string artist
        property url imageUrl
        property int durationMs: 0
        property double playedAtMs: 0
        property string artistId
        property string albumId
        readonly property bool playing: uri !== "" && uri === Player.trackUri
        width: ListView.view ? ListView.view.width : parent.width
        height: Theme.size.trackRow
        activeFocusOnTab: true

        Accessible.role: Accessible.ListItem
        Accessible.name: qsTr("%1, %2").arg(title).arg(artist)
        Accessible.onPressAction: History.playEntry(entryId)

        function openMenu(atPointer) {
            forceActiveFocus()
            historyMenu.openFor(rrow, atPointer)
        }
        function activate() { History.playEntry(entryId) }
        Keys.onReturnPressed: activate()
        Keys.onEnterPressed: activate()
        Keys.onSpacePressed: activate()
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onMenuPressed: openMenu(false)
        Keys.onPressed: event => {
            if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
                openMenu(false); event.accepted = true
            }
        }

        readonly property bool surfaceShown: activeFocus || rowHover.hovered
        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: Theme.space.sm
            anchors.rightMargin: Theme.space.sm
            radius: Theme.radius.md
            color: rrow.activeFocus ? Theme.accentSoft : Theme.surface2
            opacity: rrow.surfaceShown ? 1 : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, rrow.surfaceShown ? root.instantDuration : root.fastDuration); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            border.width: rrow.activeFocus ? Theme.stroke.focus : 0
            border.color: Theme.focusRing
        }
        HoverHandler { id: rowHover; cursorShape: Qt.PointingHandCursor }
        TapHandler {
            acceptedButtons: Qt.LeftButton
            onTapped: { rrow.forceActiveFocus(Qt.MouseFocusReason); History.playEntry(rrow.entryId) }
        }
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: rrow.openMenu(true)
        }

        Rectangle {
            id: rthumb
            anchors.left: parent.left
            anchors.leftMargin: Theme.space.lg
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.panel.queueThumb
            height: width
            radius: Theme.radius.sm
            color: Theme.surface3
            IconImage {
                anchors.centerIn: parent
                visible: rart.status !== Image.Ready
                source: Qt.resolvedUrl("../../icons/music_note.svg")
                sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                color: Theme.textTertiary
            }
            Image {
                id: rart
                anchors.fill: parent
                source: rrow.imageUrl
                asynchronous: true
                sourceSize: Qt.size(Theme.panel.queueThumb * 2, Theme.panel.queueThumb * 2)
                fillMode: Image.PreserveAspectCrop
                visible: status === Image.Ready
            }
        }
        Column {
            anchors.left: rthumb.right
            anchors.leftMargin: Theme.space.md
            anchors.right: rtrailing.left
            anchors.rightMargin: Theme.space.sm
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.xxs
            Text {
                width: parent.width
                text: rrow.title
                elide: Text.ElideRight
                font.family: Theme.family
                font.pixelSize: Theme.type.body
                font.weight: Theme.type.semibold
                color: rrow.playing ? Theme.accent : Theme.text
            }
            Text {
                width: parent.width
                text: qsTr("%1 • %2").arg(rrow.artist).arg(root.ago(rrow.playedAtMs))
                elide: Text.ElideRight
                font.family: Theme.family
                font.pixelSize: Theme.type.caption
                color: Theme.textSecondary
            }
        }
        Item {
            id: rtrailing
            anchors.right: parent.right
            anchors.rightMargin: Theme.space.md
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.size.iconButton + Theme.space.xs
            height: Theme.size.iconButton
            readonly property bool showMore: rowHover.hovered || rrow.activeFocus || rmore.activeFocus
            Text {
                anchors.right: parent.right
                anchors.rightMargin: Theme.space.xs
                anchors.verticalCenter: parent.verticalCenter
                opacity: rtrailing.showMore ? 0 : 1
                visible: opacity > 0
                Behavior on opacity {
                    NumberAnimation { duration: rtrailing.showMore ? root.instantDuration : root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                text: root.fmt(rrow.durationMs)
                font.family: Theme.family
                font.pixelSize: Theme.type.micro
                font.features: Theme.tabularFeatures
                color: Theme.textTertiary
            }
            IconButton {
                id: rmore
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                opacity: rtrailing.showMore ? 1 : 0
                visible: opacity > 0
                Behavior on opacity {
                    NumberAnimation { duration: rtrailing.showMore ? root.instantDuration : root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                text: qsTr("Opciones de %1").arg(rrow.title)
                iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                iconSize: Theme.icon.md
                ink: Theme.textSecondary
                onClicked: rrow.openMenu(false)
            }
        }
    }

    // B3: placeholder row while the queue is "loading" (static shapes; no shimmer, no timer).
    component SkeletonRow: Item {
        width: ListView.view ? ListView.view.width : parent.width
        height: Theme.size.trackRow
        Rectangle {
            id: sthumb
            x: Theme.space.xl
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.panel.queueThumb
            height: width
            radius: Theme.radius.sm
            color: Theme.surface2
        }
        Column {
            anchors.left: sthumb.right
            anchors.leftMargin: Theme.space.md
            anchors.verticalCenter: parent.verticalCenter
            spacing: Theme.space.xs
            Rectangle { width: Theme.panel.queueThumb * 3; height: Theme.space.sm; radius: height / 2; color: Theme.surface2 }
            Rectangle { width: Theme.panel.queueThumb * 2; height: Theme.space.sm; radius: height / 2; color: Theme.surface2 }
        }
    }

    RectangularShadow {
        anchors.fill: panel
        radius: Theme.radius.lg
        offset.y: Theme.elevation.raised.offsetY
        blur: Theme.elevation.raised.blur
        spread: Theme.elevation.raised.spread
        color: Qt.rgba(0, 0, 0, Theme.elevation.raised.alpha)
    }

    Rectangle {
        id: panel
        anchors.fill: parent
        anchors.rightMargin: Theme.queue.inset
        radius: Theme.radius.lg
        // User 2026-10-10: over Now Playing it takes the room's colour (the cover's veil, almost
        // opaque: the text keeps the contrast Rust picked for that veil) instead of a grey card.
        color: root.onCover ? Qt.rgba(Theme.cover.veil.r, Theme.cover.veil.g, Theme.cover.veil.b, Theme.queue.coverAlpha)
                            : Theme.surface1
        border.width: Theme.stroke.hairline
        border.color: root.onCover ? Theme.cover.pill : Theme.hairline

        // Swallows clicks so they do not reach the view underneath.
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

        // ---------------------------------------------------------------- header
        Item {
            id: header
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: Theme.size.iconButton + 2 * Theme.space.md

            // N4: "Cola" / "Recientes" pills. The accent fill slides from one to the other
            // (x of a small item, `fast`; instant in Modo rendimiento) and the content crossfades.
            Item {
                id: tabBar
                objectName: "queueTabs"
                anchors.left: parent.left
                anchors.leftMargin: Theme.space.lg
                anchors.verticalCenter: parent.verticalCenter
                width: 2 * root.pillWidth + Theme.space.xs
                height: Theme.artistPage.chipHeight
                Accessible.role: Accessible.PageTabList
                Accessible.name: qsTr("Cola y recientes")
                Rectangle {
                    id: tabIndicator
                    width: root.pillWidth
                    height: parent.height
                    radius: height / 2
                    color: Theme.accentSoft
                    x: root.showRecents ? root.pillWidth + Theme.space.xs : 0
                    Behavior on x {
                        NumberAnimation { duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
                    }
                }
                TabPill {
                    id: queueTab
                    objectName: "queueTabQueue"
                    x: 0
                    text: qsTr("Cola")
                    selected: !root.showRecents
                    onClicked: root.selectTab(0)
                    Keys.onRightPressed: { root.selectTab(1); recentsTab.forceActiveFocus(Qt.TabFocusReason) }
                }
                TabPill {
                    id: recentsTab
                    objectName: "queueTabRecents"
                    x: root.pillWidth + Theme.space.xs
                    text: qsTr("Recientes")
                    selected: root.showRecents
                    onClicked: root.selectTab(1)
                    Keys.onLeftPressed: { root.selectTab(0); queueTab.forceActiveFocus(Qt.TabFocusReason) }
                }
            }
            IconButton {
                id: panelMore
                objectName: "queuePanelMore"
                visible: !root.showRecents
                anchors.right: closeButton.left
                anchors.rightMargin: Theme.space.xs
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Opciones de la cola")
                NowPlayingToolTip { button: panelMore }
                iconSource: Qt.resolvedUrl("../../icons/more_horiz.svg")
                iconSize: Theme.icon.md
                ink: Theme.textSecondary
                onClicked: panelMenu.popup(panelMore, panelMore.width - panelMenu.width, panelMore.height)
            }
            IconButton {
                id: closeButton
                anchors.right: parent.right
                anchors.rightMargin: Theme.space.md
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Cerrar la cola")
                NowPlayingToolTip { button: closeButton }
                iconSource: Qt.resolvedUrl("../../icons/close.svg")
                iconSize: Theme.icon.sm
                ink: Theme.textSecondary
                onClicked: root.close()
            }
        }


        // ---------------------------------------------------------------- list
        ListView {
            id: list
            objectName: "queueList"
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: header.bottom
            anchors.bottom: parent.bottom
            anchors.bottomMargin: Theme.space.sm
            clip: true
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds
            // While a row is being dragged the list must not flick under the pointer.
            interactive: root.dragFrom < 0 && !root.handleHovered
            // B3: with the queue "unavailable" the "A continuación" section is hidden, with no notice.
            model: Player.queueStatus === "unavailable" ? null : Player.queue
            activeFocusOnTab: !root.showRecents
            keyNavigationEnabled: true
            Keys.onReturnPressed: if (currentItem) currentItem.activate()
            Keys.onEnterPressed: if (currentItem) currentItem.activate()
            Keys.onSpacePressed: if (currentItem) currentItem.activate()
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            Keys.onMenuPressed: if (currentItem) currentItem.openMenu(false)
            Keys.onPressed: event => {
                if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier) && currentItem) {
                    currentItem.openMenu(false); event.accepted = true
                }
            }
            // N4: crossfade with "Recientes" (opacity only, `fast`); out of the tab order once hidden.
            opacity: root.showRecents ? 0 : 1
            visible: opacity > 0
            enabled: !root.showRecents
            Behavior on opacity {
                NumberAnimation { duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }

            Accessible.role: Accessible.List
            Accessible.name: qsTr("A continuación")

            // C7: a dropped/moved row glides to its slot and the rows it passes make room
            // (y of the delegates, fast, easeOut); a removed row fades out (fast, easeIn) while
            // the rest close the gap. No relayout. NumberAnimation, not Y/OpacityAnimator: in
            // view transitions the Animators did not pick up the transition's from/to.
            move: Transition {
                NumberAnimation { property: "y"; duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            displaced: Transition {
                NumberAnimation { property: "y"; duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            remove: Transition {
                NumberAnimation { property: "opacity"; to: 0; duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeIn }
            }

            // "Sonando ahora" + the section title for what follows.
            header: Column {
                width: list.width
                spacing: 0

                Text {
                    x: Theme.space.xl
                    height: implicitHeight + Theme.space.sm
                    text: qsTr("Sonando ahora")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.caption
                    font.weight: Theme.type.semibold
                    color: Theme.textSecondary
                }
                QueueRow {
                    visible: Player.hasSession && Player.title !== ""
                    width: list.width
                    height: visible ? Theme.size.trackRow : 0
                    title: Player.title
                    artist: Player.artist
                    imageUrl: Player.coverUrl
                    durationMs: Player.durationMs
                    current: true
                }
                Text {
                    visible: !(Player.hasSession && Player.title !== "")
                    x: Theme.space.xl
                    height: visible ? implicitHeight + Theme.space.md : 0
                    text: qsTr("No suena nada")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.body
                    color: Theme.textTertiary
                }
                Item { width: 1; height: Theme.space.lg }
                Item {
                    objectName: "queueNextTitle"
                    visible: Player.queueStatus !== "unavailable"
                    width: parent.width
                    height: visible ? nextTitle.implicitHeight + Theme.space.xs : 0
                    Text {
                        id: nextTitle
                        x: Theme.space.xl
                        text: qsTr("A continuación")
                        font.family: Theme.family
                        font.pixelSize: Theme.type.caption
                        font.weight: Theme.type.semibold
                        color: Theme.textSecondary
                    }
                }
                Item { width: 1; height: Theme.space.xs }
            }

            delegate: QueueRow {
                objectName: "queueRow" + index
                required property var model
                index: model.index
                uri: model.uri
                title: model.title
                artist: model.artist
                imageUrl: model.imageUrl
                durationMs: model.durationMs
                liked: model.liked
                artistId: model.artistId !== undefined ? model.artistId : ""
                albumId: model.albumId !== undefined ? model.albumId : ""
                ListView.onPooled: if (rowMenu.row === this) rowMenu.close()
                ListView.onReused: opacity = 1   // undo the remove transition's fade

            }

            // Insertion line while dragging (in content coordinates, so it scrolls with rows).
            Rectangle {
                parent: list.contentItem
                visible: root.dropBoundary >= 0
                x: Theme.queue.dropLineInset
                width: list.width - 2 * Theme.queue.dropLineInset
                height: Theme.queue.dropLine
                radius: height / 2
                color: Theme.queue.dropColor
                y: list.originY + (list.headerItem ? list.headerItem.height : 0)
                   + root.dropBoundary * Theme.size.trackRow - height / 2
                z: 2
            }

            // B3: "loading" -> discreet skeleton; "ready" with no rows -> the empty text;
            // "unavailable" -> nothing (the section title above is hidden too).
            footer: Item {
                width: list.width
                readonly property bool skeleton: list.count === 0 && Player.queueStatus === "loading"
                readonly property bool empty: list.count === 0 && Player.queueStatus === "ready"
                readonly property bool failed: Player.queueStatus === "unavailable" && Player.queueError !== ""
                height: skeleton ? 3 * Theme.size.trackRow : empty || failed ? emptyText.implicitHeight + 2 * Theme.space.lg : 0
                Column {
                    objectName: "queueSkeleton"
                    visible: parent.skeleton
                    width: parent.width
                    SkeletonRow {}
                    SkeletonRow {}
                    SkeletonRow {}
                }
                Text {
                    id: emptyText
                    objectName: "queueEmptyText"
                    visible: parent.empty || parent.failed
                    x: Theme.space.xl
                    width: parent.width - 2 * Theme.space.xl
                    wrapMode: Text.WordWrap
                    text: parent.failed ? qsTr("No se pudo cargar la cola. Vuelve a abrir el panel para reintentar.") : qsTr("No hay nada en la cola. Añade canciones desde tu biblioteca.")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.body
                    lineHeight: Theme.type.bodyLineHeight
                    color: Theme.textTertiary
                }
            }
        }

        // ---------------------------------------------------------------- recents (N4)
        // History.model: local plays, newest first. Batches of 50 arrive as the end nears; new
        // rows fade in (opacity only). Empty: the list hides and a short friendly text shows.
        ListView {
            id: recentsList
            objectName: "recentsList"
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: header.bottom
            anchors.bottom: parent.bottom
            anchors.bottomMargin: Theme.space.sm
            clip: true
            reuseItems: true
            boundsBehavior: Flickable.StopAtBounds
            model: History.model
            activeFocusOnTab: root.showRecents
            keyNavigationEnabled: true
            Keys.onReturnPressed: if (currentItem) currentItem.activate()
            Keys.onEnterPressed: if (currentItem) currentItem.activate()
            Keys.onSpacePressed: if (currentItem) currentItem.activate()
            Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
            opacity: root.showRecents && count > 0 ? 1 : 0
            visible: opacity > 0
            enabled: root.showRecents
            Behavior on opacity {
                NumberAnimation { duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }

            Accessible.role: Accessible.List
            Accessible.name: qsTr("Recientes")

            onContentYChanged: root.maybeLoadMore()
            onCountChanged: { root.maybeLoadMore(); if (count === 0) root.moreRequestedAt = -1 }
            onHeightChanged: root.maybeLoadMore()

            // The first batch and each later one fade in (rows are created on arrival).
            populate: Transition {
                NumberAnimation { property: "opacity"; from: 0; to: 1; duration: root.baseDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            add: Transition {
                NumberAnimation { property: "opacity"; from: 0; to: 1; duration: root.baseDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }

            header: Item { width: 1; height: Theme.space.xs }
            delegate: RecentRow {
                required property var model
                entryId: model.entryId
                uri: model.uri
                title: model.title
                artist: model.artist
                imageUrl: model.imageUrl
                durationMs: model.durationMs
                playedAtMs: model.playedAtMs
                artistId: model.artistId !== undefined ? model.artistId : ""
                albumId: model.albumId !== undefined ? model.albumId : ""
                ListView.onPooled: if (historyMenu.row === this) historyMenu.close()
                ListView.onReused: opacity = 1
            }
            // "Cargando…" only while a batch is on its way (discreet, no spinner, no timer).
            footer: Item {
                width: recentsList.width
                height: History.loading ? loadingText.implicitHeight + 2 * Theme.space.md : 0
                Text {
                    id: loadingText
                    anchors.centerIn: parent
                    visible: History.loading
                    text: qsTr("Cargando…")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.caption
                    color: Theme.textTertiary
                }
            }
        }
        Text {
            objectName: "recentsEmpty"
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: header.bottom
            anchors.topMargin: Theme.space.lg
            anchors.leftMargin: Theme.space.xl
            anchors.rightMargin: Theme.space.xl
            opacity: root.showRecents && recentsList.count === 0 ? 1 : 0
            visible: opacity > 0
            Behavior on opacity {
                NumberAnimation { duration: root.fastDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
            }
            wrapMode: Text.WordWrap
            text: History.loading ? qsTr("Cargando…") : History.error !== "" ? qsTr("No se pudo cargar lo que escuchaste. Vuelve a abrir el panel para reintentar.") : qsTr("Lo que escuches con Velo aparecerá aquí.")
            font.family: Theme.family
            font.pixelSize: Theme.type.body
            lineHeight: Theme.type.bodyLineHeight
            color: Theme.textTertiary
        }
    }

    // G4: drag the left edge to resize; double click = default width.
    ResizeHandle {
        objectName: "queueResizeHandle"
        x: -width / 2
        height: parent.height
        z: 10
        visible: root.opened
        growsRight: false
        value: root.width
        minimum: Theme.panel.queueMinWidth
        maximum: root.widthCeiling
        onResized: w => root.dragWidth = w
        onCommitted: w => { layoutStore.queueWidth = w / Theme.zoomFactor; root.dragWidth = -1 }
        onResetRequested: { layoutStore.queueWidth = 0; root.dragWidth = -1 }
    }

    // While it fades out it no longer belongs to the layout (the page is already wide again):
    // it swallows the pointer so a stray click cannot act on rows that are going away.
    MouseArea {
        anchors.fill: parent
        z: 1000
        visible: !root.opened
        acceptedButtons: Qt.AllButtons
        hoverEnabled: true
        onWheel: wheel => wheel.accepted = true
    }
}
