import QtQuick
import QtQuick.Templates as T
import QtQuick.Controls.impl
import Velo.Backend as Backend
import "../../theme"
import "../controls"
import ".."

// S9/N2: Ajustes as a page of the library (route Nav.show("settings", "<section>")), like
// Spotifast: title, «Buscar en los ajustes», then a left-aligned reading column of section
// cards. Each row: title + description on the left, its control on the right.
// - The search filters ROWS by label, description, keywords and the titles of their card and
//   section; empty cards and sections hide; nothing left shows an accessible empty state.
// - Every control writes straight to Backend.Settings (the bridge persists it), except the
//   proxy draft, confirmed together by applyProxy(), and the engine settings, applied by
//   reconnectPlayback() («Se aplicará al reconectar»).
// - No Timer. Sizes measured once on entry (refreshCacheSize). The equalizer curve is painted
//   once per change (Canvas, no animation).
// Motion (/animate, Theme.motion; Modo rendimiento / Reducir movimiento make it instant):
// sections rise in on entry with a short stagger; rows that come back while filtering fade in
// (instant, no movement: typing drives it); preset pills settle on press, selection fades. Opacity and
// transforms only: the filter itself re-lays the column out once, never animated.
FocusScope {
    id: root
    objectName: "settingsPage"
    Keys.onShortcutOverride: event => {
        const w = root.Window.window
        if (event.key === Qt.Key_Space && w && w.activeFocusItem && w.activeFocusItem.activeFocusOnTab)
            event.accepted = true
    }

    // Section to bring into view when the page opens ("account", "playback", "appearance",
    // "proxy", "equalizer", "storage", "about"); "" = top.
    property string focusSection: ""

    // ---- search -----------------------------------------------------------------------
    readonly property bool isSettingsPage: true          // rows find this page by it
    // R3: the personal app wizard starts folded, unless the shared app reports a problem.
    property bool wizardOpen: Backend.Account.appWarning !== ""
    readonly property string query: fold(searchField.text)
    // Rows animate when they come back after the first layout (not on the page's entrance).
    property bool revealArmed: false
    function fold(text) {
        let s = String(text || "").toLowerCase().trim()
        if (typeof s.normalize === "function")
            s = s.normalize("NFD").replace(/[̀-ͯ]/g, "")
        return s.replace(/\s+/g, " ")
    }
    function matches(text) {
        return query === "" || fold(text).indexOf(query) >= 0
    }
    readonly property int shownSections: {
        let n = 0
        const c = sectionsColumn.children
        for (let i = 0; i < c.length; ++i)
            if (c[i].shown === true) ++n
        return n
    }

    // R3 (P2-11): index of sections. A rail at the left (icon over label, as in ref_02) when the
    // page is wide enough, a row of chips under the search otherwise. A click scrolls to the
    // section immediately and the entry of the section
    // at the top of the view is marked. Sections hidden by the search leave the index too.
    readonly property var sectionIndex: [
        { key: "account", label: qsTr("Cuenta"), icon: "account_circle" },
        { key: "playback", label: qsTr("Reproducción"), icon: "play_circle" },
        { key: "appearance", label: qsTr("Apariencia"), icon: "palette" },
        { key: "proxy", label: qsTr("Proxy"), icon: "computer" },
        { key: "equalizer", label: qsTr("Ecualizador"), icon: "graphic_eq" },
        { key: "storage", label: qsTr("Almacenamiento"), icon: "storage" },
        { key: "about", label: qsTr("Acerca de"), icon: "info" }
    ]
    readonly property bool railMode: width >= Theme.library.settingsRailMinPage
    // Composition: rail + reading column form one block centred in the page (it hugs the left
    // edge only when the page is narrower than the block). The scrollbar and the wheel still
    // belong to the whole width of the page.
    readonly property int railGap: Theme.space.xl
    readonly property real blockWidth: railMode ? Theme.library.settingsRail + railGap + Theme.settingsPage.column : 0
    readonly property real blockX: railMode ? Math.max(0, Math.round((width - blockWidth - Theme.space.lg) / 2)) : 0
    readonly property real columnX: railMode ? blockX + Theme.library.settingsRail + railGap : 0
    function sectionItem(key) {
        const c = sectionsColumn.children
        for (let i = 0; i < c.length; ++i)
            if (c[i].sectionKey === key) return c[i]
        return null
    }
    function sectionShown(key) {
        const item = sectionItem(key)
        return item !== null && item.shown === true
    }
    // The section whose title is at (or last passed) the top of the view.
    readonly property string currentSection: {
        const c = sectionsColumn.children
        const top = flick.contentY + Theme.settingsPage.sectionGap
        let current = ""
        for (let i = 0; i < c.length; ++i) {
            if (c[i].shown !== true) continue
            if (current === "" || sectionsColumn.y + c[i].y <= top) current = c[i].sectionKey
        }
        if (flick.atYEnd && flick.contentHeight > flick.height) {
            for (let i = c.length - 1; i >= 0; --i)
                if (c[i].shown === true) return c[i].sectionKey
        }
        return current
    }
    function targetY(key) {
        const item = sectionItem(key)
        if (item === null) return -1
        const y = item.mapToItem(flick.contentItem, 0, 0).y - Theme.space.lg
        return Math.max(0, Math.min(y, flick.contentHeight - flick.height))
    }
    function scrollTo(key) {
        const to = targetY(key)
        if (to < 0) return
        navAnim.stop(); wheel.stop()
        flick.contentY = to
    }
    // Index click (occasional): the page glides to the section so the jump reads as a move.
    // navTarget pins the selected entry to the destination while it travels, so the scroll-spy
    // does not flash the sections in between. Opening the page at a section stays instant
    // (scrollTo), as does the keyboard focus reveal.
    property string navTarget: ""
    readonly property string selectedSection: navTarget !== "" ? navTarget : currentSection
    function glideTo(key) {
        const to = targetY(key)
        if (to < 0) return
        wheel.stop()
        if (Theme.motion.slow <= 0 || Math.abs(to - flick.contentY) > 3 * flick.height) {
            navAnim.stop(); navTarget = ""; flick.contentY = to
            return
        }
        navTarget = key
        navAnim.stop()
        navAnim.to = to
        navAnim.start()
    }
    NumberAnimation {
        id: navAnim
        target: flick
        property: "contentY"
        duration: Math.max(1, Theme.motion.slow)
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.motion.easeInOut
        onStopped: root.navTarget = ""
    }
    function revealControl(item) {
        if (!item) return
        let ancestor = item
        while (ancestor && ancestor !== flick.contentItem) ancestor = ancestor.parent
        if (!ancestor) return
        const top = item.mapToItem(flick.contentItem, 0, 0).y
        const bottom = top + item.height
        navAnim.stop(); wheel.stop()
        const to = top < flick.contentY ? top : bottom > flick.contentY + flick.height ? bottom - flick.height : flick.contentY
        flick.contentY = Math.max(0, Math.min(to, Math.max(0, flick.contentHeight - flick.height)))
    }
    Connections {
        target: root.Window.window
        function onActiveFocusItemChanged() { root.revealControl(root.Window.window.activeFocusItem) }
    }

    // One entry of the index. Rail: icon over a short label; chip: label only.
    component IndexEntry: T.AbstractButton {
        id: entry
        property string sectionKey: ""
        property url iconSource
        property bool selected: false
        property bool rail: true
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        implicitWidth: rail ? Theme.library.settingsRail : entryText.implicitWidth + 2 * Theme.space.md
        implicitHeight: rail ? Theme.library.settingsRailItem : Theme.library.chipHeight
        Accessible.role: Accessible.Button
        Accessible.name: text
        Accessible.description: selected ? qsTr("Sección actual") : ""
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        scale: pressed ? Theme.feel.pressScale : 1
        background: Rectangle {
            radius: entry.rail ? Theme.radius.md : height / 2
            color: entry.hovered ? Theme.surface2 : (entry.rail ? Theme.library.clearPaint : Theme.surface1)
            border.width: entry.visualFocus ? Theme.stroke.focus : (entry.rail ? 0 : Theme.stroke.hairline)
            border.color: entry.visualFocus ? Theme.focusRing : Theme.hairline
            // Selection is immediate; no motion while navigating settings in pause.
            Rectangle {
                anchors.fill: parent
                radius: parent.radius
                color: Theme.accentSoft
                opacity: entry.selected ? 1 : 0
            }
        }
        contentItem: Item {
            Column {
                anchors.centerIn: parent
                spacing: Theme.space.xs
                IconImage {
                    visible: entry.rail
                    anchors.horizontalCenter: parent.horizontalCenter
                    source: entry.iconSource
                    sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                    color: entry.selected ? Theme.accent : Theme.textSecondary
                }
                Text {
                    id: entryText
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(implicitWidth, entry.width - 2 * Theme.space.xs)
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    text: entry.text
                    font.family: Theme.family
                    font.pixelSize: entry.rail ? Theme.type.micro : Theme.type.caption
                    font.weight: entry.selected ? Theme.type.semibold : Theme.type.medium
                    color: entry.selected ? Theme.text : Theme.textSecondary
                }
            }
        }
    }
    onFocusSectionChanged: if (focusSection !== "") Qt.callLater(scrollTo, focusSection)

    Component.onCompleted: {
        Backend.Settings.refreshCacheSize()   // both caches, measured in the background (no polling)
        if (focusSection !== "") Qt.callLater(scrollTo, focusSection)
        entrance.start()
    }

    // Presets: contract names stay the values; these are only their visible labels.
    function presetLabel(name) {
        switch (name) {
        case "Flat": return qsTr("Plano")
        case "Classical": return qsTr("Clásica")
        case "Club": return qsTr("Club")
        case "Dance": return qsTr("Dance")
        case "Full Bass": return qsTr("Graves")
        case "Full Bass & Treble": return qsTr("Graves y agudos")
        case "Full Treble": return qsTr("Agudos")
        case "Laptop Speakers/Headphones": return qsTr("Portátil y auriculares")
        case "Large Hall": return qsTr("Sala grande")
        case "Live": return qsTr("En vivo")
        case "Party": return qsTr("Fiesta")
        case "Pop": return qsTr("Pop")
        case "Reggae": return qsTr("Reggae")
        case "Rock": return qsTr("Rock")
        case "Ska": return qsTr("Ska")
        case "Soft": return qsTr("Suave")
        case "Soft Rock": return qsTr("Rock suave")
        case "Techno": return qsTr("Techno")
        case "Bass Booster": return qsTr("Más graves")
        case "Bass Reducer": return qsTr("Menos graves")
        case "Treble Booster": return qsTr("Más agudos")
        case "Vocal Booster": return qsTr("Más voces")
        case "Small Speakers": return qsTr("Parlantes pequeños")
        case "Spoken Word": return qsTr("Voz hablada")
        case "Loudness": return qsTr("Sonoridad")
        case "Night Listening": return qsTr("Escucha nocturna")
        case "Custom": return qsTr("Personalizado")
        }
        return name
    }
    readonly property var eqBandLabels: [qsTr("Pre"), "60", "170", "310", "600", qsTr("1K"), qsTr("3K"),
                                         qsTr("6K"), qsTr("12K"), qsTr("14K"), qsTr("16K")]
    function dbText(db) {
        const v = Math.round(db * 2) / 2
        return v > 0 ? qsTr("+%1").arg(v) : String(v)
    }
    function mbText(mb) {
        return mb >= 1024 ? qsTr("%1 GB").arg(Math.round(mb / 102.4) / 10) : qsTr("%1 MB").arg(mb)
    }

    // ---- entrance: sections rise in turn (render-thread Animators, one shot) --------------
    SequentialAnimation {
        id: entrance
        ScriptAction {
            script: {
                const c = sectionsColumn.children
                for (let i = 0; i < c.length; ++i)
                    if (c[i].enter) c[i].enter()
            }
        }
        // Rows only start animating on filter changes once the first layout is in place.
        PropertyAction { target: root; property: "revealArmed"; value: true }
    }

    // ---- confirmations ------------------------------------------------------------------
    LibraryConfirmDialog {
        id: signOutDialog; objectName: "settingsSignOutConfirm"
        title: qsTr("¿Cerrar sesión?")
        message: qsTr("Se detendrá la música y se borrarán las autorizaciones de Spotify de este PC.")
        confirmText: qsTr("Cerrar sesión")
        onConfirmed: Backend.Account.signOut()
    }
    property bool historyCleared: false
    LibraryConfirmDialog {
        id: clearHistoryDialog; objectName: "settingsClearHistoryConfirm"
        title: qsTr("¿Borrar el historial?")
        message: qsTr("Se borrarán las canciones que escuchaste en Velo de este PC. Tu cuenta de Spotify no cambia.")
        confirmText: qsTr("Borrar historial")
        onConfirmed: Backend.Settings.clearHistory()
    }
    Connections {
        target: Backend.History
        ignoreUnknownSignals: true
        function onCleared() { root.historyCleared = true }
        function onChanged() { if (Backend.History.count > 0) root.historyCleared = false }
    }

    Shortcut {
        sequences: [StandardKey.Find]
        onActivated: { searchField.forceActiveFocus(); searchField.selectAll() }
    }

    // ===================================================================== building blocks
    // Inline components do not see this file's ids: rows find the page (and the titles of the
    // card and section around them) by walking up their parents.
    component SettingRow: Item {
        id: row
        property string label: ""
        property string hint: ""
        property string keywords: ""
        property bool available: true          // false: never shown (no data / not applicable)
        property bool stacked: false           // control under the text, full width
        property bool dimmed: false            // shown but inactive (e.g. its master switch is off)
        property color hintColor: Theme.textTertiary
        default property alias control: slot.data
        // Layout: stacked on request, or when the control would take most of the row (a narrow
        // window or a large zoom), so the text never gets squeezed to a sliver.
        readonly property bool stack: stacked || slot.childrenRect.width > row.width * Theme.settingsPage.controlShare

        function scan() {
            let text = "", host = null
            for (let p = row.parent; p; p = p.parent) {
                if (p.settingsSearchText !== undefined) text += " " + p.settingsSearchText
                if (p.isSettingsPage === true) { host = p; break }
            }
            return { host: host, text: text }
        }
        readonly property var scope: scan()
        readonly property bool matched: available
            && (scope.host === null || scope.host.matches(label + " " + hint + " " + keywords + scope.text))

        visible: matched
        width: parent ? parent.width : 0
        implicitHeight: stack ? texts.height + (texts.height > 0 ? Theme.space.md : 0) + slot.height
                                : Math.max(Theme.settingsPage.rowMinHeight, texts.height, slot.height)
        height: implicitHeight
        opacity: dimmed ? Theme.library.disabledOpacity : 1
        // Master switch dims/restores dependent rows softly. One owner per property: the filter
        // reveal below owns opacity while it runs, so this stays off then.
        Behavior on opacity {
            enabled: !reveal.running
            OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }

        Accessible.role: Accessible.Grouping
        Accessible.name: label

        // Hairline between the rows of a card (not above the first visible one): it sits in the
        // middle of the gap the card's Column leaves above this row.
        readonly property bool leading: {
            const c = row.parent ? row.parent.children : []
            for (let i = 0; i < c.length; ++i) {
                if (c[i] === row) return true
                if (c[i].visible && c[i].matched === true) return false
            }
            return true
        }
        Rectangle {
            visible: !row.leading
            x: 0; width: row.width
            y: -Math.round(Theme.settingsPage.rowGap / 2)
            height: Theme.stroke.hairline
            color: Theme.hairline
        }

        Column {
            id: texts
            x: 0
            y: row.stack ? 0 : Math.round((row.height - height) / 2)
            width: row.stack ? row.width : Math.max(0, row.width - slot.width - Theme.space.xl)
            spacing: Theme.space.xxs
            Text {
                width: parent.width
                visible: row.label !== ""
                text: row.label
                wrapMode: Text.WordWrap
                font.family: Theme.family
                font.pixelSize: Theme.type.body
                font.weight: Theme.type.medium
                color: Theme.text
            }
            Text {
                width: parent.width
                visible: row.hint !== ""
                text: row.hint
                wrapMode: Text.WordWrap
                font.family: Theme.family
                font.pixelSize: Theme.type.caption
                lineHeight: Theme.type.bodyLineHeight
                color: row.hintColor
            }
        }
        Item {
            id: slot
            x: row.stack ? 0 : row.width - width
            y: row.stack ? texts.height + (texts.height > 0 ? Theme.space.md : 0) : Math.round((row.height - height) / 2)
            width: row.stacked ? row.width : childrenRect.width
            height: childrenRect.height
        }

        // Filter motion: typing drives it (keystroke-frequent), so only a near-imperceptible
        // fade (instant, ease-out, render thread) on a row that comes back; no movement.
        onMatchedChanged: {
            if (!matched || !scope.host || !scope.host.revealArmed || Theme.motion.instant <= 0) return
            reveal.restart()
        }
        OpacityAnimator {
            id: reveal
            target: row
            from: 0
            to: row.dimmed ? Theme.library.disabledOpacity : 1
            duration: Math.max(1, Theme.motion.instant)
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.motion.easeOut
            onStopped: row.opacity = Qt.binding(() => row.dimmed ? Theme.library.disabledOpacity : 1)
        }
    }

    // A paragraph inside a card (explanations, notices). Filtered like a row.
    component Note: Text {
        id: note
        property bool available: true
        property bool notice: false            // soft accent plate («Se aplicará al reconectar»)
        function scan() {
            let text = "", host = null
            for (let p = note.parent; p; p = p.parent) {
                if (p.settingsSearchText !== undefined) text += " " + p.settingsSearchText
                if (p.isSettingsPage === true) { host = p; break }
            }
            return { host: host, text: text }
        }
        readonly property var scope: scan()
        readonly property bool matched: available && (scope.host === null || scope.host.matches(text + scope.text))
        visible: matched
        width: parent ? parent.width : 0
        wrapMode: Text.WordWrap
        font.family: Theme.family
        font.pixelSize: Theme.type.caption
        lineHeight: Theme.type.bodyLineHeight
        color: notice ? Theme.text : Theme.textTertiary
        leftPadding: notice ? Theme.space.md : 0
        rightPadding: notice ? Theme.space.md : 0
        topPadding: notice ? Theme.space.sm : 0
        bottomPadding: notice ? Theme.space.sm : 0
        Rectangle {
            visible: note.notice
            z: -1
            anchors.fill: parent
            radius: Theme.radius.md
            color: Theme.settingsPage.warnFill
        }
    }

    // A rounded card of rows; `heading` (optional) titles a group inside a section.
    component Card: Item {
        id: card
        property string heading: ""
        readonly property string settingsSearchText: heading
        default property alias rows: rowsColumn.data
        readonly property bool shown: {
            const c = rowsColumn.children
            for (let i = 0; i < c.length; ++i)
                if (c[i].matched === true) return true
            return false
        }
        visible: shown
        width: parent ? parent.width : 0
        height: shown ? rowsColumn.height + 2 * Theme.settingsPage.cardPadding : 0

        Rectangle {
            anchors.fill: parent
            radius: Theme.settingsPage.cardRadius
            color: Theme.settingsPage.cardFill
            border.width: Theme.stroke.hairline
            border.color: Theme.settingsPage.cardBorder
        }
        Column {
            id: rowsColumn
            x: Theme.settingsPage.cardPadding
            y: Theme.settingsPage.cardPadding
            width: card.width - 2 * Theme.settingsPage.cardPadding
            spacing: Theme.settingsPage.rowGap
            Text {
                // `matched: false`: a heading never keeps its card visible on its own.
                readonly property bool matched: false
                visible: card.heading !== ""
                width: parent.width
                text: card.heading
                font.family: Theme.family
                font.pixelSize: Theme.type.caption
                font.weight: Theme.type.semibold
                font.capitalization: Font.AllUppercase
                font.letterSpacing: Theme.stroke.hairline
                color: Theme.textSecondary
                Accessible.role: Accessible.Heading
            }
        }
    }

    // A section: its title over one or more cards. Hidden when none of its cards has rows.
    component Section: Item {
        id: section
        property string sectionKey: ""
        property string title: ""
        property int order: 0
        readonly property string icon: ({ account: "account_circle", playback: "play_circle", appearance: "palette",
                                          proxy: "computer", equalizer: "graphic_eq", storage: "storage", about: "info" })[sectionKey] || ""
        readonly property string settingsSearchText: title
        default property alias cards: cardsColumn.data
        readonly property bool shown: {
            const c = cardsColumn.children
            for (let i = 0; i < c.length; ++i)
                if (c[i].shown === true) return true
            return false
        }
        visible: shown
        width: parent ? parent.width : 0
        height: shown ? body.height : 0

        // Entrance (once, when the page opens): fade + rise by entrance.cardShift after the
        // section's stagger. Animators on the inner body (the Column places `section`).
        function enter() {
            if (Theme.motion.base <= 0) return
            body.opacity = 0
            body.y = Theme.entrance.cardShift
            hold.duration = Math.max(1, order < Theme.entrance.staggerMax ? order * Theme.entrance.stagger : 0)
            sectionIn.start()
        }
        SequentialAnimation {
            id: sectionIn
            PauseAnimation { id: hold }
            ParallelAnimation {
                OpacityAnimator { target: body; from: 0; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                YAnimator { target: body; from: Theme.entrance.cardShift; to: 0; duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }

        Column {
            id: body
            width: section.width
            spacing: Theme.space.md
            Row {
                width: parent.width
                spacing: Theme.space.md
                bottomPadding: Theme.space.xxs
                Rectangle {   // section badge: the same glyph as the index entry
                    width: Theme.settingsPage.badge; height: width
                    anchors.verticalCenter: titleText.verticalCenter
                    radius: Theme.radius.md
                    color: Theme.accentSoft
                    IconImage {
                        anchors.centerIn: parent
                        source: section.icon !== "" ? Qt.resolvedUrl("../../icons/" + section.icon + ".svg") : ""
                        sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                        color: Theme.accent
                    }
                }
                Text {
                    id: titleText
                    width: parent.width - Theme.settingsPage.badge - Theme.space.md
                    text: section.title
                    elide: Text.ElideRight
                    font.family: Theme.family
                    font.pixelSize: Theme.type.title
                    font.weight: Theme.type.bold
                    font.letterSpacing: Theme.type.tracking(Theme.type.title)
                    color: Theme.text
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
            }
            Column {
                id: cardsColumn
                width: parent.width
                spacing: Theme.space.md
            }
        }
    }

    // A small label for numbers next to controls (MB, %, dB).
    component Figure: Text {
        font.family: Theme.family
        font.pixelSize: Theme.type.body
        font.weight: Theme.type.semibold
        font.features: Theme.tabularFeatures
        color: Theme.text
    }

    // Preset pill: selected = soft accent fill (fades), press settles (scale).
    component PresetPill: T.AbstractButton {
        id: pill
        property bool selected: false
        implicitWidth: pillLabel.implicitWidth + 2 * Theme.space.md
        implicitHeight: Theme.settingsPage.presetPill
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        padding: 0
        Accessible.role: Accessible.RadioButton
        Accessible.name: text
        Accessible.checked: selected
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        scale: pressed ? Theme.feel.pressScale : 1
        Behavior on scale {
            ScaleAnimator { duration: pill.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        background: Rectangle {
            radius: height / 2
            color: pill.hovered && pill.enabled ? Theme.surface3 : Theme.surface2
            border.width: pill.visualFocus ? Theme.stroke.focus : Theme.stroke.hairline
            border.color: pill.visualFocus ? Theme.focusRing : Theme.hairline
            Rectangle {
                anchors.fill: parent
                radius: parent.radius
                color: Theme.accentSoft
                border.width: Theme.stroke.hairline
                border.color: Theme.accent
                opacity: pill.selected ? 1 : 0
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
            }
        }
        contentItem: Text {
            id: pillLabel
            text: pill.text
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            font.family: Theme.family
            font.pixelSize: Theme.type.caption
            font.weight: pill.selected ? Theme.type.semibold : Theme.type.medium
            color: pill.selected ? Theme.accent : Theme.textSecondary
        }
    }

    // Vertical equalizer slider (−12…+12 dB): the fill grows from 0 dB toward the knob.
    component EqSlider: T.Slider {
        id: eq
        property string text: ""
        orientation: Qt.Vertical
        from: -Theme.settingsPage.eqRangeDb
        to: Theme.settingsPage.eqRangeDb
        stepSize: Theme.settingsPage.eqStepDb
        snapMode: T.Slider.SnapAlways
        implicitWidth: Theme.size.iconButton
        implicitHeight: Theme.settingsPage.eqSliderHeight
        focusPolicy: Qt.StrongFocus
        hoverEnabled: true
        padding: 0
        Accessible.name: text
        Accessible.role: Accessible.Slider
        readonly property real zeroY: topPadding + (1 - (0 - from) / (to - from)) * (availableHeight - handle.height) + handle.height / 2
        readonly property real knobY: handle.y + handle.height / 2
        background: Item {
            x: eq.leftPadding + eq.availableWidth / 2 - width / 2
            width: Theme.stroke.progress
            height: eq.height
            Rectangle {
                anchors.fill: parent
                radius: width / 2
                color: Theme.surface3
            }
            Rectangle {
                width: parent.width
                radius: width / 2
                y: Math.min(eq.zeroY, eq.knobY)
                height: Math.abs(eq.zeroY - eq.knobY)
                color: eq.enabled ? Theme.accent : Theme.textDisabled
            }
        }
        handle: Rectangle {
            x: eq.leftPadding + eq.availableWidth / 2 - width / 2
            y: eq.topPadding + eq.visualPosition * (eq.availableHeight - height)
            width: Theme.control.sliderKnobHeight + Theme.space.xs
            height: Theme.space.sm - Theme.space.xxs
            radius: height / 2
            color: eq.enabled ? (eq.hovered || eq.pressed ? Theme.accentHover : Theme.accent) : Theme.textDisabled
            border.width: eq.visualFocus ? Theme.stroke.focus : 0
            border.color: Theme.text
            scale: eq.pressed ? Theme.feel.knobGrow : 1
            Behavior on scale {
                ScaleAnimator { duration: eq.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
        }
    }

    // ===================================================================== index (rail)
    Column {
        id: rail
        objectName: "settingsIndex"
        x: root.blockX
        z: 1
        visible: root.railMode
        width: Theme.library.settingsRail
        spacing: Theme.space.xs
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Secciones de Ajustes")
        Repeater {
            id: railRepeater
            model: root.sectionIndex
            delegate: IndexEntry {
                required property var modelData
                required property int index
                objectName: "settingsIndex_" + modelData.key
                rail: true
                sectionKey: modelData.key
                text: modelData.label
                iconSource: Qt.resolvedUrl("../../icons/" + modelData.icon + ".svg")
                visible: root.sectionShown(modelData.key)
                selected: root.selectedSection === modelData.key
                onClicked: root.glideTo(modelData.key)
                function step(by) {
                    for (let i = index + by; i >= 0 && i < railRepeater.count; i += by) {
                        const next = railRepeater.itemAt(i)
                        if (next && next.visible) { next.forceActiveFocus(Qt.TabFocusReason); return }
                    }
                }
                Keys.onUpPressed: step(-1)
                Keys.onDownPressed: step(1)
            }
        }
    }

    Flow {
        id: chipIndex
        objectName: "settingsIndexChips"
        visible: !root.railMode
        width: parent.width
        spacing: Theme.space.xs
        Accessible.role: Accessible.List
        Accessible.name: qsTr("Secciones de Ajustes")
        Repeater {
            model: root.railMode ? [] : root.sectionIndex
            delegate: IndexEntry {
                required property var modelData
                objectName: "settingsIndexChip_" + modelData.key
                rail: false
                sectionKey: modelData.key
                text: modelData.label
                visible: root.sectionShown(modelData.key)
                selected: root.selectedSection === modelData.key
                onClicked: root.glideTo(modelData.key)
            }
        }
    }

    // ===================================================================== page
    Flickable {
        id: flick
        objectName: "settingsViewport"
        anchors.top: root.railMode ? parent.top : chipIndex.bottom
        anchors.topMargin: root.railMode ? 0 : Theme.space.lg
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        anchors.left: parent.left
        contentWidth: width
        contentHeight: page.height + Theme.space.xxl
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        T.ScrollBar.vertical: LibraryScrollBar {}
        // The user takes over (wheel, drag, scrollbar): the glide yields at once.
        onMovingChanged: if (moving) { navAnim.stop(); wheel.stop() }

        // The mouse wheel moves ~3 rows per notch and glides (components/WheelScroll.qml).
        WheelScroll { id: wheel; rowHeight: Theme.settingsPage.rowMinHeight }

        Column {
            id: page
            x: root.columnX
            width: Math.min(flick.width - root.columnX - Theme.space.lg, Theme.settingsPage.column)
            spacing: Theme.settingsPage.sectionGap

            // ---- header: title + search ----------------------------------------------
            Column {
                width: parent.width
                spacing: Theme.space.lg
                Text {
                    text: qsTr("Ajustes")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.display
                    font.weight: Theme.type.heavy
                    font.letterSpacing: Theme.type.tracking(Theme.type.display)
                    color: Theme.text
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                VTextField {
                    id: searchField
                    objectName: "settingsSearch"
                    width: Math.min(parent.width, Theme.settingsPage.searchWidth)
                    placeholderText: qsTr("Buscar en los ajustes")
                    leftPadding: Theme.space.md + Theme.icon.sm + Theme.space.sm
                    rightPadding: clearSearch.visible ? clearSearch.width + Theme.space.sm : Theme.space.md
                    Keys.onEscapePressed: event => {
                        if (text !== "") { clear(); event.accepted = true }
                        else event.accepted = false
                    }
                    IconImage {
                        x: Theme.space.md
                        anchors.verticalCenter: parent.verticalCenter
                        source: Qt.resolvedUrl("../../icons/search.svg")
                        sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                        color: Theme.textTertiary
                    }
                    IconButton {
                        id: clearSearch
                        visible: searchField.text !== ""
                        anchors.right: parent.right
                        anchors.rightMargin: Theme.space.xxs
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.size.iconButton - Theme.space.xs
                        height: width
                        text: qsTr("Borrar búsqueda")
                        iconSource: Qt.resolvedUrl("../../icons/close.svg")
                        iconSize: Theme.icon.sm
                        ink: Theme.textSecondary
                        onClicked: { searchField.clear(); searchField.forceActiveFocus() }
                    }
                }
            }

            // ---- empty search ----------------------------------------------------------
            Column {
                visible: root.shownSections === 0
                width: parent.width
                spacing: Theme.space.sm
                Accessible.role: Accessible.StaticText
                Accessible.name: emptyTitle.text
                Text {
                    id: emptyTitle
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: qsTr("No hay ajustes que coincidan con «%1»").arg(searchField.text.trim())
                    font.family: Theme.family
                    font.pixelSize: Theme.type.title
                    font.weight: Theme.type.semibold
                    color: Theme.text
                }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: qsTr("Prueba con otra palabra, como «tema», «caché» o «proxy».")
                    font.family: Theme.family
                    font.pixelSize: Theme.type.body
                    color: Theme.textSecondary
                }
            }

            Column {
                id: sectionsColumn
                width: parent.width
                spacing: Theme.settingsPage.sectionGap

                // ===================================================== Cuenta
                Section {
                    sectionKey: "account"
                    title: qsTr("Cuenta")
                    order: 0
                    Card {
                        SettingRow {
                            id: profileRow
                            label: Backend.Account.signedIn ? Backend.Account.displayName : qsTr("Sin sesión")
                            hint: Backend.Account.signedIn && Backend.Account.isPremium ? qsTr("Spotify Premium") : ""
                            keywords: qsTr("perfil usuario cerrar sesión iniciar sesión salir")
                            VButton {
                                kind: Backend.Account.signedIn ? "danger" : "accent"
                                text: Backend.Account.signedIn ? qsTr("Cerrar sesión") : qsTr("Iniciar sesión con Spotify")
                                iconSource: Qt.resolvedUrl(Backend.Account.signedIn ? "../../icons/logout.svg" : "../../icons/login.svg")
                                enabled: !Backend.Account.busy
                                onClicked: Backend.Account.signedIn ? signOutDialog.open() : Backend.Account.signIn()
                            }
                        }
                        // Velo's own audio (E2): a one-time second login; then this PC is a Connect device.
                        SettingRow {
                            id: audioRow
                            property string failure: ""
                            available: Backend.Account.signedIn
                            label: qsTr("Reproducción en Velo")
                            keywords: qsTr("audio activar dispositivo")
                            hintColor: audioRow.failure !== "" ? Theme.danger : Theme.textTertiary
                            hint: audioRow.failure !== "" ? audioRow.failure
                                : Backend.Account.audioReady ? qsTr("Velo suena en este PC sin abrir la app de Spotify.")
                                : Backend.Account.audioBusy ? qsTr("Termina el inicio de sesión en el navegador.")
                                : qsTr("Escucha en Velo sin abrir la app de Spotify. Se pide permiso una sola vez en el navegador.")
                            Connections {
                                target: Backend.Account
                                function onAudioFailed(reason) { audioRow.failure = reason }
                                function onAudioBusyChanged() { if (Backend.Account.audioBusy) audioRow.failure = "" }
                            }
                            Row {
                                spacing: Theme.space.xs
                                IconImage {
                                    visible: Backend.Account.audioReady
                                    anchors.verticalCenter: parent.verticalCenter
                                    source: Qt.resolvedUrl("../../icons/check.svg")
                                    sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                                    color: Theme.accent
                                }
                                Text {
                                    visible: Backend.Account.audioReady
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Activa")
                                    font.family: Theme.family
                                    font.pixelSize: Theme.type.body
                                    font.weight: Theme.type.semibold
                                    color: Theme.accent
                                }
                                VButton {
                                    visible: !Backend.Account.audioReady
                                    kind: "accent"
                                    text: Backend.Account.audioBusy ? qsTr("Esperando…") : qsTr("Activar")
                                    enabled: !Backend.Account.audioBusy
                                    onClicked: Backend.Account.enableAudio()
                                }
                            }
                        }
                    }
                    Card {
                        heading: qsTr("App de Spotify")
                        Note {
                            color: Backend.Account.appWarning !== "" ? Theme.danger : Theme.textSecondary
                            text: Backend.Account.appWarning !== "" ? Backend.Account.appWarning
                                : Backend.Account.clientId === "" ? qsTr("Cuenta de Spotify: un solo inicio de sesión para música y biblioteca.")
                                : qsTr("App personal activa.")
                        }
                        // R3 (P2-11): a fallback for few: one row with «Configurar…»; the six steps
                        // unfold under it (opened already when the shared app reports a problem,
                        // and while searching for it).
                        SettingRow {
                            label: Backend.Account.clientId === "" ? qsTr("Tu propia app de Spotify") : qsTr("Cambiar de app personal")
                            hint: qsTr("Conecta una app personal con tu cuenta desde el navegador.")
                            keywords: qsTr("client id app personal propia spotify developer")
                            VButton {
                                objectName: "settingsPersonalAppToggle"
                                text: root.wizardOpen ? qsTr("Ocultar los pasos") : qsTr("Configurar…")
                                Accessible.description: root.wizardOpen ? qsTr("Pasos desplegados") : qsTr("Pasos plegados")
                                onClicked: root.wizardOpen = !root.wizardOpen
                            }
                        }
                        SettingRow {
                            id: personalWizardRow
                            objectName: "settingsPersonalAppWizard"
                            stacked: true
                            available: root.wizardOpen || Backend.Account.busy || (root.query !== "" && root.matches(keywords))
                            keywords: qsTr("client id app personal propia spotify developer")
                            Loader {
                                width: parent.width
                                active: personalWizardRow.available
                                sourceComponent: PersonalAppWizard { width: parent.width }
                            }
                        }
                        SettingRow {
                            available: Backend.Account.clientId !== ""
                            label: qsTr("Volver a la cuenta de Spotify")
                            hint: qsTr("Deja de usar tu app personal.")
                            VButton {
                                text: qsTr("Usar solo mi cuenta de Spotify")
                                enabled: !Backend.Account.busy
                                onClicked: Backend.Account.useSpotifyAccount()
                            }
                        }
                    }
                }

                // ===================================================== Reproducción
                Section {
                    sectionKey: "playback"
                    title: qsTr("Reproducción en este PC")
                    order: 1
                    Card {
                        SettingRow {
                            label: qsTr("Estado")
                            keywords: qsTr("motor conexión reconectar")
                            hint: Backend.Settings.playbackStatus === "error"
                                  ? qsTr("Reconecta para volver a intentarlo.")
                                  : qsTr("Velo aparece en tus dispositivos como «%1».").arg(Backend.Settings.connectDeviceName)
                            Row {
                                spacing: Theme.space.md
                                readonly property string status: Backend.Settings.playbackStatus
                                readonly property bool busy: status === "connecting" || status === "reconnecting"
                                Rectangle {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Theme.space.sm
                                    height: width
                                    radius: width / 2
                                    color: parent.status === "ready" ? Theme.accent
                                         : parent.status === "error" ? Theme.danger : Theme.textTertiary
                                }
                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: parent.status === "ready" ? qsTr("Lista")
                                        : parent.status === "connecting" ? qsTr("Conectando…")
                                        : parent.status === "reconnecting" ? qsTr("Reconectando…")
                                        : parent.status === "error" ? qsTr("Sin conexión")
                                        : qsTr("Apagada")
                                    font.family: Theme.family
                                    font.pixelSize: Theme.type.body
                                    font.weight: Theme.type.semibold
                                    color: parent.status === "error" ? Theme.danger : Theme.text
                                }
                                VButton {
                                    objectName: "settingsReconnect"
                                    text: qsTr("Reconectar")
                                    enabled: !parent.busy
                                    onClicked: Backend.Settings.reconnectPlayback()
                                }
                            }
                        }
                        Note {
                            objectName: "settingsRestartNotice"
                            notice: true
                            available: Backend.Settings.playbackRestartRequired === true
                            text: qsTr("Se aplicará al reconectar.")
                        }
                        SettingRow {
                            label: qsTr("Nombre del dispositivo")
                            hint: qsTr("Cómo se ve este PC en Spotify Connect.")
                            keywords: qsTr("connect nombre")
                            VTextField {
                                width: Theme.settingsPage.fieldWidth
                                text: Backend.Settings.connectDeviceName
                                maximumLength: Theme.library.deviceNameMax
                                placeholderText: qsTr("Nombre del dispositivo")
                                onEditingFinished: {
                                    const name = text.trim()
                                    if (name !== "" && name !== Backend.Settings.connectDeviceName)
                                        Backend.Settings.connectDeviceName = name
                                    else text = Backend.Settings.connectDeviceName
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Calidad de audio")
                            hint: qsTr("Más calidad usa más datos.")
                            keywords: qsTr("kbps bitrate sonido")
                            VSegmented {
                                name: qsTr("Calidad de audio")
                                model: [
                                    { value: "96", label: qsTr("96 kbps") },
                                    { value: "160", label: qsTr("160 kbps") },
                                    { value: "320", label: qsTr("320 kbps") }
                                ]
                                current: String(Backend.Settings.audioQuality)
                                onActivated: value => Backend.Settings.audioQuality = parseInt(value)
                            }
                        }
                        SettingRow {
                            label: qsTr("Normalizar volumen")
                            hint: qsTr("Todas las canciones suenan a un volumen parecido.")
                            VToggle {
                                text: qsTr("Normalizar volumen")
                                checked: Backend.Settings.normalizeVolume
                                onToggled: Backend.Settings.normalizeVolume = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Reproducción automática")
                            hint: qsTr("Al terminar la lista, sigue con canciones parecidas.")
                            keywords: qsTr("autoplay")
                            VToggle {
                                text: qsTr("Reproducción automática")
                                checked: Backend.Settings.autoplay
                                onToggled: Backend.Settings.autoplay = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Ocultar actividad") // English: Hide activity
                            hint: qsTr("No guardar en Recientes de Velo y ocultar tu actividad en Spotify")
                            keywords: qsTr("privacidad historial recientes")
                            VToggle {
                                text: qsTr("Ocultar actividad")
                                checked: Backend.Settings.privateSession
                                onToggled: Backend.Settings.privateSession = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Sin pausas entre canciones")
                            hint: qsTr("Una canción empieza justo cuando termina la anterior.")
                            keywords: qsTr("gapless")
                            VToggle {
                                text: qsTr("Sin pausas entre canciones")
                                checked: Backend.Settings.gapless
                                onToggled: Backend.Settings.gapless = checked
                            }
                        }
                        SettingRow {
                            objectName: "mixAutoRow"
                            label: qsTr("Mixear automáticamente")
                            hint: Backend.Settings.gapless
                                ? qsTr("Mezcla el final de cada canción con el comienzo de la siguiente. También puedes activarlo por playlist.")
                                : qsTr("Requiere activar «Sin pausas entre canciones».")
                            keywords: qsTr("mixear mezcla fundido crossfade")
                            dimmed: !Backend.Settings.gapless
                            VToggle {
                                objectName: "mixAutoToggle"
                                text: qsTr("Mixear automáticamente")
                                enabled: Backend.Settings.gapless
                                checked: Backend.Settings.mixAuto
                                onToggled: Backend.Settings.mixAuto = checked
                            }
                        }
                        SettingRow {
                            objectName: "mixSecondsRow"
                            label: qsTr("Duración de la mezcla")
                            hint: qsTr("Cuántos segundos se solapan dos canciones al mixear.")
                            keywords: qsTr("mixear segundos fundido crossfade")
                            dimmed: !Backend.Settings.gapless
                            Row {
                                spacing: Theme.space.md
                                VSlider {
                                    objectName: "mixSecondsSlider"
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Theme.settingsPage.fieldWidth
                                    text: qsTr("Duración de la mezcla")
                                    enabled: Backend.Settings.gapless
                                    from: 1
                                    to: 12
                                    stepSize: 1
                                    value: Backend.Settings.mixSeconds
                                    onMoved: Backend.Settings.mixSeconds = Math.round(value)
                                }
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("%1 s").arg(Backend.Settings.mixSeconds)
                                    color: Theme.textSecondary
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Búfer de salida")
                            hint: qsTr("Más corto responde antes; más largo aguanta mejor los cortes.")
                            keywords: qsTr("buffer latencia ms")
                            VSegmented {
                                name: qsTr("Búfer de salida")
                                model: [
                                    { value: "200", label: qsTr("200 ms") },
                                    { value: "100", label: qsTr("100 ms") },
                                    { value: "50", label: qsTr("50 ms") }
                                ]
                                current: String(Backend.Settings.outputBufferMs)
                                onActivated: value => Backend.Settings.outputBufferMs = parseInt(value)
                            }
                        }
                        SettingRow {
                            label: qsTr("Al cerrar, seguir en la bandeja")
                            hint: qsTr("Cerrar la ventana la oculta y la música sigue. Sal desde el icono de la bandeja.")
                            keywords: qsTr("cerrar bandeja segundo plano")
                            VToggle {
                                text: qsTr("Al cerrar, seguir en la bandeja")
                                checked: Backend.Settings.closeToTray
                                onToggled: Backend.Settings.closeToTray = checked
                            }
                        }
                    }
                }

                // ===================================================== Apariencia
                Section {
                    sectionKey: "appearance"
                    title: qsTr("Apariencia")
                    order: 2
                    Card {
                        SettingRow {
                            label: qsTr("Tema")
                            keywords: qsTr("claro oscuro modo")
                            VSegmented {
                                name: qsTr("Tema")
                                model: [
                                    { value: "system", label: qsTr("Sistema") },
                                    { value: "light", label: qsTr("Claro") },
                                    { value: "dark", label: qsTr("Oscuro") }
                                ]
                                current: Backend.Settings.theme
                                onActivated: value => Backend.Settings.theme = value
                            }
                        }
                        SettingRow {
                            label: qsTr("Idioma")
                            keywords: qsTr("language")
                            VSegmented {
                                name: qsTr("Idioma")
                                model: [
                                    { value: "system", label: qsTr("Sistema") },
                                    { value: "es", label: qsTr("Español") },
                                    { value: "en", label: qsTr("English") }
                                ]
                                current: Backend.Settings.language
                                onActivated: value => Backend.Settings.language = value
                            }
                        }
                        SettingRow {
                            objectName: "settingsZoomRow"
                            label: qsTr("Tamaño de la interfaz")
                            hint: qsTr("También con Ctrl + Más, Ctrl + Menos y Ctrl + 0.")
                            keywords: qsTr("zoom escala texto grande pequeño")
                            Row {
                                spacing: Theme.space.xs
                                readonly property real zoom: Backend.Settings.uiZoom
                                IconButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Reducir la interfaz")
                                    iconSource: Qt.resolvedUrl("../../icons/remove.svg")
                                    iconSize: Theme.icon.sm
                                    enabled: parent.zoom > Theme.zoomMin + 0.001
                                    onClicked: Backend.Settings.uiZoom = Theme.nextZoom(parent.zoom, -1)
                                }
                                T.AbstractButton {
                                    id: zoomReset
                                    anchors.verticalCenter: parent.verticalCenter
                                    implicitWidth: zoomFigure.implicitWidth + 2 * Theme.space.sm
                                    implicitHeight: Theme.size.iconButton
                                    hoverEnabled: true
                                    focusPolicy: Qt.StrongFocus
                                    text: qsTr("Restablecer el tamaño (%1 %)").arg(Math.round(parent.zoom * 100))
                                    Accessible.name: text
                                    Accessible.role: Accessible.Button
                                    onClicked: Backend.Settings.uiZoom = Theme.nextZoom(parent.zoom, 0)
                                    background: Rectangle {
                                        radius: Theme.radius.md
                                        color: zoomReset.hovered ? Theme.surface2 : Theme.library.clearPaint
                                        border.width: zoomReset.visualFocus ? Theme.stroke.focus : 0
                                        border.color: Theme.focusRing
                                    }
                                    contentItem: Figure {
                                        id: zoomFigure
                                        horizontalAlignment: Text.AlignHCenter
                                        verticalAlignment: Text.AlignVCenter
                                        text: qsTr("%1 %").arg(Math.round(zoomReset.parent.zoom * 100))
                                    }
                                }
                                IconButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Agrandar la interfaz")
                                    iconSource: Qt.resolvedUrl("../../icons/add.svg")
                                    iconSize: Theme.icon.sm
                                    enabled: parent.zoom < Theme.zoomMax - 0.001
                                    onClicked: Backend.Settings.uiZoom = Theme.nextZoom(parent.zoom, 1)
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Controles de ventana")
                            hint: qsTr("Semáforos a la izquierda o botones a la derecha.")
                            VSegmented {
                                name: qsTr("Controles de ventana")
                                model: [
                                    { value: "mac", label: qsTr("Mac") },
                                    { value: "windows", label: qsTr("Windows") }
                                ]
                                current: Backend.Settings.windowControls
                                onActivated: value => Backend.Settings.windowControls = value
                            }
                        }
                        SettingRow {
                            label: qsTr("Barra lateral compacta")
                            hint: qsTr("Tu biblioteca sin carátulas, en filas más bajas.")
                            keywords: qsTr("sidebar")
                            VToggle {
                                text: qsTr("Barra lateral compacta")
                                checked: Backend.Settings.sidebarCompact
                                onToggled: Backend.Settings.sidebarCompact = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Listas de canciones compactas")
                            hint: qsTr("Vista inicial de las listas; cada lista puede cambiarla con «Ver como».")
                            keywords: qsTr("compacta filas canciones")
                            VToggle {
                                text: qsTr("Listas de canciones compactas")
                                checked: Backend.Settings.compactTracks
                                onToggled: Backend.Settings.compactTracks = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Visualizador en la barra de reproducción")
                            hint: qsTr("Detrás de los controles de la biblioteca, mientras suena una canción.")
                            keywords: qsTr("espectro onda barras")
                            VSegmented {
                                name: qsTr("Visualizador en la barra de reproducción")
                                model: [
                                    { value: "off", label: qsTr("Desactivado") },
                                    { value: "bars", label: qsTr("Espectro") },
                                    { value: "wave", label: qsTr("Forma de onda") }
                                ]
                                current: Backend.Settings.playerBarVisualizer
                                onActivated: value => Backend.Settings.playerBarVisualizer = value
                            }
                        }
                    }
                    Card {
                        heading: qsTr("Reproduciendo ahora")
                        SettingRow {
                            label: qsTr("Mostrar el visualizador")
                            hint: qsTr("Solo decoración: no cambia el sonido. Usa algo de CPU y batería mientras suena.")
                            VToggle {
                                text: qsTr("Mostrar el visualizador")
                                checked: Backend.Visualizer.enabled
                                onToggled: Backend.Visualizer.enabled = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Carátula")
                            VSegmented {
                                name: qsTr("Carátula")
                                model: [
                                    { value: "normal", label: qsTr("Cuadrada") },
                                    { value: "circular", label: qsTr("Circular") }
                                ]
                                current: Backend.Visualizer.style
                                onActivated: value => Backend.Visualizer.style = value
                            }
                        }
                        SettingRow {
                            label: qsTr("Banda inferior")
                            dimmed: !Backend.Visualizer.enabled
                            VToggle {
                                text: qsTr("Banda inferior")
                                enabled: Backend.Visualizer.enabled
                                checked: Backend.Visualizer.band
                                onToggled: Backend.Visualizer.band = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Forma del visualizador")
                            dimmed: !Backend.Visualizer.enabled
                            VSegmented {
                                name: qsTr("Forma del visualizador")
                                enabled: Backend.Visualizer.enabled
                                model: [
                                    { value: "bars", label: qsTr("Barras") },
                                    { value: "wave", label: qsTr("Ondas") }
                                ]
                                current: Backend.Visualizer.shape
                                onActivated: value => Backend.Visualizer.shape = value
                            }
                        }
                        SettingRow {
                            label: qsTr("Sensibilidad")
                            dimmed: !Backend.Visualizer.enabled
                            Row {
                                spacing: Theme.space.md
                                VSlider {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Theme.settingsPage.fieldWidth
                                    text: qsTr("Sensibilidad")
                                    enabled: Backend.Visualizer.enabled
                                    from: Theme.library.sensitivityMin
                                    to: Theme.library.sensitivityMax
                                    stepSize: Theme.library.sensitivityStep
                                    value: Backend.Visualizer.sensitivity * 100
                                    onMoved: Backend.Visualizer.sensitivity = value / 100
                                }
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("%1 %").arg(Math.round(Backend.Visualizer.sensitivity * 100))
                                    color: Theme.textSecondary
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Fondo con la carátula")
                            hint: qsTr("Muestra la carátula desenfocada detrás de la canción.")
                            keywords: qsTr("blur fondo")
                            VToggle {
                                text: qsTr("Fondo con la carátula")
                                checked: Backend.Settings.coverBlur
                                onToggled: Backend.Settings.coverBlur = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Deriva lenta")
                            hint: qsTr("Mueve despacio el fondo. Desactívala para ahorrar batería.")
                            keywords: qsTr("fondo movimiento")
                            VToggle {
                                text: qsTr("Deriva lenta")
                                checked: Backend.Settings.drift
                                onToggled: Backend.Settings.drift = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Velocidad del fondo")
                            hint: qsTr("Qué tan rápido gira y se mueve la carátula difuminada.")
                            dimmed: !Backend.Settings.drift
                            Row {
                                spacing: Theme.space.md
                                VSlider {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Theme.settingsPage.fieldWidth
                                    text: qsTr("Velocidad del fondo")
                                    enabled: Backend.Settings.drift
                                    from: 0; to: 100; stepSize: 5
                                    value: Backend.Settings.hypnoticSpeed
                                    onMoved: Backend.Settings.hypnoticSpeed = Math.round(value)
                                }
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("%1 %").arg(Backend.Settings.hypnoticSpeed)
                                    color: Theme.textSecondary
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Intensidad del fondo")
                            hint: qsTr("Cuánto se mezclan y brillan los colores de la carátula.")
                            dimmed: !Backend.Settings.drift
                            Row {
                                spacing: Theme.space.md
                                VSlider {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Theme.settingsPage.fieldWidth
                                    text: qsTr("Intensidad del fondo")
                                    enabled: Backend.Settings.drift
                                    from: 0; to: 100; stepSize: 5
                                    value: Backend.Settings.hypnoticIntensity
                                    onMoved: Backend.Settings.hypnoticIntensity = Math.round(value)
                                }
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("%1 %").arg(Backend.Settings.hypnoticIntensity)
                                    color: Theme.textSecondary
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Teñir la interfaz")
                            hint: qsTr("Usa los colores de la carátula en los controles.")
                            keywords: qsTr("color")
                            VToggle {
                                text: qsTr("Teñir la interfaz")
                                checked: Backend.Settings.tintUi
                                onToggled: Backend.Settings.tintUi = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Mostrar letras")
                            hint: qsTr("Letras sincronizadas junto a la carátula, cuando existan.")
                            keywords: qsTr("lyrics")
                            VToggle {
                                text: qsTr("Mostrar letras")
                                checked: Backend.Settings.showLyrics
                                onToggled: Backend.Settings.showLyrics = checked
                            }
                        }
                    }
                    Card {
                        heading: qsTr("Movimiento y batería")
                        SettingRow {
                            label: qsTr("Reducir movimiento")
                            hint: qsTr("Quita desplazamientos y escalas; los fundidos suaves se mantienen.")
                            keywords: qsTr("animaciones accesibilidad")
                            VToggle {
                                text: qsTr("Reducir movimiento")
                                checked: Backend.Settings.reduceMotion
                                onToggled: Backend.Settings.reduceMotion = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Modo rendimiento")
                            hint: qsTr("Sin animaciones, fundidos ni visualizador: gasta menos CPU y batería.")
                            keywords: qsTr("animaciones cpu")
                            VToggle {
                                text: qsTr("Modo rendimiento")
                                checked: Backend.Settings.performanceMode
                                onToggled: Backend.Settings.performanceMode = checked
                            }
                        }
                        // G4: sub-option of Modo rendimiento, indented under it and only there
                        // while the mode is on (off by default: the mode turns the visualizer off).
                        SettingRow {
                            objectName: "performanceKeepVisualizerRow"
                            x: Theme.settingsPage.subIndent
                            width: parent.width - x
                            available: Backend.Settings.performanceMode
                            label: qsTr("Mantener el visualizador")
                            hint: qsTr("Sigue mostrando el visualizador con el Modo rendimiento activo.")
                            keywords: qsTr("modo rendimiento visualizador barras ola")
                            VToggle {
                                text: qsTr("Mantener el visualizador")
                                checked: Backend.Settings.performanceKeepVisualizer
                                enabled: Backend.Settings.performanceMode
                                onToggled: Backend.Settings.performanceKeepVisualizer = checked
                            }
                        }
                        SettingRow {
                            label: qsTr("Pausar efectos en segundo plano")
                            hint: Backend.Settings.performanceMode
                                  ? qsTr("Siempre activo en Modo rendimiento.")
                                  : qsTr("El visualizador se detiene cuando Velo pierde el foco.")
                            keywords: qsTr("foco animaciones")
                            VToggle {
                                text: qsTr("Pausar efectos en segundo plano")
                                checked: Backend.Settings.pauseInBackground || Backend.Settings.performanceMode
                                enabled: !Backend.Settings.performanceMode
                                onToggled: Backend.Settings.pauseInBackground = checked
                            }
                        }
                    }
                }

                // ===================================================== Proxy
                Section {
                    id: proxySection
                    sectionKey: "proxy"
                    title: qsTr("Proxy")
                    order: 3
                    readonly property var modes: {
                        const all = [
                            { value: "off", label: qsTr("Desactivado") },
                            { value: "system", label: qsTr("Sistema") },
                            { value: "http", label: qsTr("HTTP") },
                            { value: "socks5", label: qsTr("SOCKS5") }
                        ]
                        const supported = Backend.Settings.supportedProxyModes || []
                        return all.filter(m => supported.indexOf(m.value) >= 0)
                    }
                    readonly property bool manual: Backend.Settings.proxyMode === "http" || Backend.Settings.proxyMode === "socks5"
                    Card {
                        SettingRow {
                            available: proxySection.modes.length > 1
                            label: qsTr("Tipo de proxy")
                            keywords: qsTr("red conexión servidor")
                            hint: Backend.Settings.proxyMode === "system" ? qsTr("Usa la configuración de proxy de Windows.")
                                : proxySection.manual ? qsTr("Velo se conecta a Spotify a través de este servidor.")
                                : qsTr("Velo se conecta directamente.")
                            VSegmented {
                                name: qsTr("Tipo de proxy")
                                model: proxySection.modes
                                current: Backend.Settings.proxyMode
                                onActivated: value => Backend.Settings.proxyMode = value
                            }
                        }
                        SettingRow {
                            available: proxySection.modes.length > 1 && proxySection.manual
                            label: qsTr("Servidor")
                            hint: qsTr("Dirección y puerto, sin http:// delante.")
                            keywords: qsTr("host puerto dirección")
                            Row {
                                spacing: Theme.space.sm
                                VTextField {
                                    id: proxyHost
                                    width: Theme.settingsPage.fieldWidth - Theme.settingsPage.portWidth - Theme.space.sm
                                    text: Backend.Settings.proxyHost
                                    placeholderText: qsTr("proxy.ejemplo.com")
                                    Accessible.name: qsTr("Servidor proxy")
                                    inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhUrlCharactersOnly
                                    onTextEdited: Backend.Settings.proxyHost = text.trim()
                                }
                                VTextField {
                                    width: Theme.settingsPage.portWidth
                                    text: String(Backend.Settings.proxyPort)
                                    placeholderText: qsTr("Puerto")
                                    Accessible.name: qsTr("Puerto del proxy")
                                    inputMethodHints: Qt.ImhDigitsOnly
                                    validator: IntValidator { bottom: 1; top: 65535 }
                                    onTextEdited: if (acceptableInput) Backend.Settings.proxyPort = parseInt(text)
                                }
                            }
                        }
                        SettingRow {
                            available: proxySection.modes.length > 1 && proxySection.manual
                            label: qsTr("Usuario")
                            hint: qsTr("Solo si tu proxy pide iniciar sesión.")
                            VTextField {
                                width: Theme.settingsPage.fieldWidth
                                text: Backend.Settings.proxyUser
                                placeholderText: qsTr("Opcional")
                                Accessible.name: qsTr("Usuario del proxy")
                                inputMethodHints: Qt.ImhNoAutoUppercase
                                onTextEdited: Backend.Settings.proxyUser = text
                            }
                        }
                        SettingRow {
                            available: proxySection.modes.length > 1 && proxySection.manual
                            label: qsTr("Contraseña")
                            hint: Backend.Settings.proxyPasswordStored
                                  ? qsTr("Guardada en el llavero de Windows. Escribe otra para cambiarla.")
                                  : qsTr("Se guarda en el llavero de Windows, nunca en archivos.")
                            Row {
                                spacing: Theme.space.sm
                                VTextField {
                                    id: proxyPassword
                                    objectName: "settingsProxyPassword"
                                    width: Theme.settingsPage.fieldWidth
                                    echoMode: TextInput.Password
                                    placeholderText: Backend.Settings.proxyPasswordStored ? qsTr("Guardada") : qsTr("Opcional")
                                    Accessible.name: qsTr("Contraseña del proxy")
                                    inputMethodHints: Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhHiddenText
                                }
                                IconButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: Backend.Settings.proxyPasswordStored
                                    text: qsTr("Quitar la contraseña guardada")
                                    iconSource: Qt.resolvedUrl("../../icons/close.svg")
                                    iconSize: Theme.icon.sm
                                    ink: Theme.textSecondary
                                    enabled: !Backend.Settings.proxyBusy
                                    onClicked: Backend.Settings.clearProxyPassword()
                                }
                            }
                        }
                        SettingRow {
                            available: proxySection.modes.length > 1
                            label: qsTr("Aplicar el proxy")
                            hint: qsTr("Los cambios se guardan juntos al aplicar.")
                            VButton {
                                objectName: "settingsApplyProxy"
                                kind: "accent"
                                text: Backend.Settings.proxyBusy ? qsTr("Aplicando…") : qsTr("Aplicar")
                                enabled: !Backend.Settings.proxyBusy
                                onClicked: {
                                    const password = proxyPassword.text
                                    Backend.Settings.applyProxy(password, password !== "")
                                    proxyPassword.clear()   // never kept in QML longer than needed
                                }
                            }
                        }
                        Note {
                            objectName: "settingsProxyError"
                            available: proxySection.modes.length > 1 && Backend.Settings.proxyError !== ""
                            color: Theme.danger
                            text: Backend.Settings.proxyError
                        }
                    }
                }

                // ===================================================== Ecualizador
                Section {
                    id: eqSection
                    sectionKey: "equalizer"
                    title: qsTr("Ecualizador")
                    order: 4
                    readonly property bool on: Backend.Settings.eqEnabled === true
                    Card {
                        SettingRow {
                            label: qsTr("Ecualizador")
                            hint: qsTr("Ajusta graves, medios y agudos del sonido de Velo en este PC.")
                            keywords: qsTr("eq sonido graves agudos")
                            Row {
                                spacing: Theme.space.md
                                VButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Restablecer")
                                    enabled: eqSection.on
                                    onClicked: Backend.Settings.resetEq()
                                }
                                VToggle {
                                    objectName: "settingsEqToggle"
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Ecualizador")
                                    checked: eqSection.on
                                    onToggled: Backend.Settings.eqEnabled = checked
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Ajustes predefinidos")
                            keywords: (Backend.Settings.eqPresets || []).map(p => root.presetLabel(p) + " " + p).join(" ")
                            stacked: true
                            dimmed: !eqSection.on
                            Flow {
                                width: parent.width
                                spacing: Theme.space.sm
                                Accessible.role: Accessible.List
                                Accessible.name: qsTr("Ajustes predefinidos")
                                Repeater {
                                    model: Backend.Settings.eqPresets || []
                                    delegate: PresetPill {
                                        required property string modelData
                                        objectName: "settingsPreset" + modelData
                                        text: root.presetLabel(modelData)
                                        selected: Backend.Settings.eqPreset === modelData
                                        enabled: eqSection.on
                                        onClicked: Backend.Settings.setEqPreset(modelData)
                                    }
                                }
                                PresetPill {
                                    visible: Backend.Settings.eqPreset === "Custom"
                                    text: root.presetLabel("Custom")
                                    selected: true
                                    enabled: false
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Bandas")
                            hint: qsTr("Pre sube o baja todo antes del ecualizador.")
                            keywords: qsTr("hz frecuencias preamplificador")
                            stacked: true
                            dimmed: !eqSection.on
                            Flickable {
                                id: eqBlock
                                objectName: "settingsEqBlock"
                                width: parent.width
                                height: eqCurve.height + Theme.space.sm + eqSliders.height + (overflows ? Theme.space.md : 0)
                                contentWidth: eqBlock.column * 11
                                contentHeight: height
                                readonly property bool overflows: contentWidth > width + 0.5
                                interactive: overflows
                                clip: overflows
                                flickableDirection: Flickable.HorizontalFlick
                                boundsBehavior: Flickable.StopAtBounds
                                T.ScrollBar.horizontal: LibraryScrollBar {
                                    implicitHeight: Theme.space.sm
                                    policy: eqBlock.overflows ? T.ScrollBar.AlwaysOn : T.ScrollBar.AlwaysOff
                                }
                                readonly property var gains: Backend.Settings.eqGainsDb || []
                                readonly property real preamp: Backend.Settings.eqPreampDb || 0
                                readonly property real column: Math.max(Theme.library.eqColumnCompact, width / 11)
                                readonly property real range: Theme.settingsPage.eqRangeDb
                                function revealBand(band) {
                                    const left = band.mapToItem(contentItem, 0, 0).x
                                    const to = left < contentX ? left
                                        : left + band.width > contentX + width ? left + band.width - width : contentX
                                    contentX = Math.max(0, Math.min(to, Math.max(0, contentWidth - width)))
                                }
                                // A resize or a zoom (columns change width) keeps the focused band in view.
                                property Item focusedSlider: null  // last band slider that took focus
                                function revealFocused() {
                                    const slider = focusedSlider
                                    if (slider && slider.focus) revealBand(slider.parent)
                                }
                                onWidthChanged: Qt.callLater(revealFocused)
                                onContentWidthChanged: Qt.callLater(revealFocused)

                                // The response curve: band gains + preamp through the band
                                // columns (Catmull-Rom), painted once per change (no animation).
                                Canvas {
                                    id: eqCurve
                                    objectName: "settingsEqCurve"
                                    width: eqBlock.column * 11
                                    height: Theme.settingsPage.eqCurveHeight
                                    Accessible.role: Accessible.Graphic
                                    Accessible.name: qsTr("Curva del ecualizador")
                                    property int paints: 0                 // read by the preview check
                                    readonly property var inputs: [eqBlock.gains, eqBlock.preamp, Theme.dark, width, height]
                                    onInputsChanged: requestPaint()
                                    onPaint: {
                                        ++paints
                                        const ctx = getContext("2d")
                                        ctx.reset()
                                        const g = eqBlock.gains
                                        const top = Theme.stroke.progress, h = height - 2 * top
                                        const yOf = db => top + (1 - (Math.max(-eqBlock.range, Math.min(eqBlock.range, db)) + eqBlock.range) / (2 * eqBlock.range)) * h
                                        // 0 dB line
                                        ctx.strokeStyle = Theme.hairline
                                        ctx.lineWidth = Theme.stroke.hairline
                                        ctx.beginPath()
                                        ctx.moveTo(eqBlock.column, yOf(0))
                                        ctx.lineTo(width, yOf(0))
                                        ctx.stroke()
                                        if (g.length < 10) return
                                        const pts = []
                                        for (let i = 0; i < 10; ++i)
                                            pts.push({ x: eqBlock.column * (i + 1.5), y: yOf(g[i] + eqBlock.preamp) })
                                        const first = { x: eqBlock.column, y: pts[0].y }
                                        const last = { x: width, y: pts[9].y }
                                        const all = [first].concat(pts, [last])
                                        ctx.beginPath()
                                        ctx.moveTo(all[0].x, all[0].y)
                                        for (let i = 0; i < all.length - 1; ++i) {
                                            const p0 = all[Math.max(0, i - 1)], p1 = all[i], p2 = all[i + 1], p3 = all[Math.min(all.length - 1, i + 2)]
                                            ctx.bezierCurveTo(p1.x + (p2.x - p0.x) / 6, p1.y + (p2.y - p0.y) / 6,
                                                              p2.x - (p3.x - p1.x) / 6, p2.y - (p3.y - p1.y) / 6, p2.x, p2.y)
                                        }
                                        ctx.strokeStyle = Theme.accent
                                        ctx.lineWidth = Theme.stroke.progress
                                        ctx.lineJoin = "round"
                                        ctx.stroke()
                                        ctx.lineTo(last.x, yOf(0))
                                        ctx.lineTo(first.x, yOf(0))
                                        ctx.closePath()
                                        ctx.fillStyle = Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, Theme.settingsPage.eqFillAlpha)
                                        ctx.fill()
                                    }
                                }
                                Row {
                                    id: eqSliders
                                    y: eqCurve.height + Theme.space.sm
                                    Repeater {
                                        model: 11
                                        delegate: Column {
                                            id: band
                                            required property int index
                                            readonly property real db: index === 0 ? eqBlock.preamp
                                                : (eqBlock.gains.length > index - 1 ? eqBlock.gains[index - 1] : 0)
                                            width: eqBlock.column
                                            spacing: Theme.space.xs
                                            Text {
                                                width: parent.width
                                                horizontalAlignment: Text.AlignHCenter
                                                text: root.dbText(band.db)
                                                font.family: Theme.family
                                                font.pixelSize: Theme.type.micro
                                                font.features: Theme.tabularFeatures
                                                color: Theme.textSecondary
                                            }
                                            EqSlider {
                                                objectName: "settingsEqBand" + band.index
                                                anchors.horizontalCenter: parent.horizontalCenter
                                                width: Math.min(implicitWidth, band.width)
                                                text: band.index === 0 ? qsTr("Preamplificador") : qsTr("%1 Hz").arg(root.eqBandLabels[band.index])
                                                enabled: eqSection.on
                                                onFocusChanged: if (focus) {
                                                    eqBlock.focusedSlider = this
                                                    eqBlock.revealBand(band)
                                                }
                                                value: band.db
                                                onMoved: {
                                                    if (band.index === 0) Backend.Settings.eqPreampDb = value
                                                    else Backend.Settings.setEqBand(band.index - 1, value)
                                                }
                                            }
                                            Text {
                                                width: parent.width
                                                horizontalAlignment: Text.AlignHCenter
                                                text: root.eqBandLabels[band.index]
                                                font.family: Theme.family
                                                font.pixelSize: Theme.type.caption
                                                font.weight: band.index === 0 ? Theme.type.semibold : Theme.type.regular
                                                font.features: Theme.tabularFeatures
                                                color: band.index === 0 ? Theme.text : Theme.textTertiary
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ===================================================== Almacenamiento
                Section {
                    sectionKey: "storage"
                    title: qsTr("Almacenamiento")
                    order: 5
                    Card {
                        heading: qsTr("Carátulas")
                        SettingRow {
                            label: qsTr("Caché de carátulas")
                            hint: qsTr("Se guarda en disco para que Velo abra rápido y gaste menos red.")
                            keywords: qsTr("imágenes disco espacio vaciar")
                            Row {
                                spacing: Theme.space.md
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: root.mbText(Backend.Settings.usedCacheMb)
                                }
                                VButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("Vaciar")
                                    onClicked: Backend.Settings.clearCache()
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Límite de la caché de carátulas")
                            hint: qsTr("Al pasarlo, Velo borra primero las carátulas más antiguas.")
                            VSegmented {
                                name: qsTr("Límite de la caché de carátulas")
                                model: [
                                    { value: "25", label: qsTr("25 MB") },
                                    { value: "50", label: qsTr("50 MB") },
                                    { value: "100", label: qsTr("100 MB") },
                                    { value: "200", label: qsTr("200 MB") }
                                ]
                                current: String(Backend.Settings.coverCacheLimitMb)
                                onActivated: value => Backend.Settings.coverCacheLimitMb = parseInt(value)
                            }
                        }
                    }
                    Card {
                        heading: qsTr("Audio")
                        SettingRow {
                            label: qsTr("Caché de audio")
                            hint: qsTr("Guarda cifrado lo que escuchas para que vuelva a sonar antes. No son archivos de música.")
                            keywords: qsTr("disco espacio streaming")
                            VToggle {
                                text: qsTr("Caché de audio")
                                checked: Backend.Settings.audioCacheEnabled
                                onToggled: Backend.Settings.audioCacheEnabled = checked
                            }
                        }
                        SettingRow {
                            available: Backend.Settings.audioCacheEnabled || Backend.Settings.usedAudioCacheMb > 0
                            label: qsTr("Espacio usado por el audio")
                            hint: qsTr("Borrarla no cierra tu sesión.")
                            keywords: qsTr("borrar caché disco")
                            Row {
                                spacing: Theme.space.md
                                Figure {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: qsTr("%1 de %2").arg(root.mbText(Backend.Settings.usedAudioCacheMb)).arg(root.mbText(Backend.Settings.audioCacheLimitMb))
                                }
                                VButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: Backend.Settings.storageBusy ? qsTr("Borrando…") : qsTr("Borrar")
                                    enabled: !Backend.Settings.storageBusy && Backend.Settings.usedAudioCacheMb > 0
                                    onClicked: Backend.Settings.clearAudioCache()
                                }
                            }
                        }
                        SettingRow {
                            available: Backend.Settings.audioCacheEnabled
                            label: qsTr("Límite de la caché de audio")
                            keywords: qsTr("espacio disco")
                            VSegmented {
                                name: qsTr("Límite de la caché de audio")
                                model: [
                                    { value: "512", label: qsTr("512 MB") },
                                    { value: "1024", label: qsTr("1 GB") },
                                    { value: "4096", label: qsTr("4 GB") }
                                ]
                                current: String(Backend.Settings.audioCacheLimitMb)
                                onActivated: value => Backend.Settings.audioCacheLimitMb = parseInt(value)
                            }
                        }
                    }
                    Card {
                        heading: qsTr("Privacidad")
                        SettingRow {
                            objectName: "settingsHistoryRow"
                            label: qsTr("Historial de reproducción")
                            hint: root.historyCleared ? qsTr("Historial borrado.")
                                : qsTr("Se guarda solo en este PC; nunca se sube.")
                            keywords: qsTr("recientes borrar escuchado")
                            VButton {
                                kind: "danger"
                                text: qsTr("Borrar historial")
                                enabled: !Backend.Settings.storageBusy && Backend.History.count > 0
                                onClicked: clearHistoryDialog.open()
                            }
                        }
                        SettingRow {
                            label: qsTr("Inicio de sesión")
                            hint: qsTr("Velo guarda tu sesión cifrada en el llavero de Windows, nunca en archivos.")
                            keywords: qsTr("llavero credenciales keyring contraseña")
                            Row {
                                spacing: Theme.space.xs
                                IconImage {
                                    visible: Backend.Settings.credentialsStored
                                    anchors.verticalCenter: parent.verticalCenter
                                    source: Qt.resolvedUrl("../../icons/check.svg")
                                    sourceSize: Qt.size(Theme.icon.sm, Theme.icon.sm)
                                    color: Theme.accent
                                }
                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: Backend.Settings.credentialsStored ? qsTr("Guardado") : qsTr("No guardado")
                                    font.family: Theme.family
                                    font.pixelSize: Theme.type.body
                                    font.weight: Theme.type.semibold
                                    color: Backend.Settings.credentialsStored ? Theme.accent : Theme.textSecondary
                                }
                            }
                        }
                        SettingRow {
                            label: qsTr("Carpeta de configuración")
                            hint: qsTr("Donde Velo guarda tus ajustes en este PC.")
                            VButton {
                                text: qsTr("Abrir carpeta")
                                iconSource: Qt.resolvedUrl("../../icons/open_in_new.svg")
                                onClicked: Backend.Settings.openConfigFolder()
                            }
                        }
                    }
                }

                // ===================================================== Acerca de
                Section {
                    sectionKey: "about"
                    title: qsTr("Acerca de")
                    order: 6
                    Card {
                        SettingRow {
                            label: Qt.application.name
                            hint: qsTr("Versión %1").arg(Backend.Updater.currentVersion)
                            keywords: qsTr("versión logo")
                            Image {
                                source: Qt.resolvedUrl("../../icons/velo.svg")
                                sourceSize: Qt.size(Theme.settingsPage.logo, Theme.settingsPage.logo)
                                width: Theme.settingsPage.logo
                                height: Theme.settingsPage.logo
                                asynchronous: true
                                Accessible.ignored: true
                            }
                        }
                        SettingRow {
                            label: qsTr("Buscar actualizaciones al iniciar")
                            keywords: qsTr("actualizar versión automática")
                            VToggle {
                                objectName: "settingsCheckUpdates"
                                checked: Backend.Settings.checkUpdates
                                onToggled: Backend.Settings.checkUpdates = checked
                            }
                        }
                        SettingRow {
                            id: updateCheckRow
                            objectName: "settingsUpdateResult"
                            property bool waiting: false
                            property string result: ""
                            label: qsTr("Actualizaciones")
                            keywords: qsTr("actualizar versión buscar")
                            hint: waiting ? qsTr("Buscando…") : result
                            hintColor: Backend.Updater.error !== "" ? Theme.danger : Theme.textTertiary
                            Connections {
                                target: Backend.Updater
                                function onCheckFinished(newer) {
                                    updateCheckRow.waiting = false
                                    updateCheckRow.result = Backend.Updater.error !== "" ? Backend.Updater.error
                                        : newer ? qsTr("Velo %1 disponible").arg(Backend.Updater.latestVersion)
                                        : qsTr("Tienes la última versión")
                                }
                            }
                            VButton {
                                objectName: "settingsCheckNow"
                                text: qsTr("Buscar ahora")
                                enabled: !updateCheckRow.waiting && Backend.Updater.state !== "downloading" && Backend.Updater.state !== "ready"
                                onClicked: { updateCheckRow.waiting = true; Backend.Updater.checkNow() }
                            }
                        }
                        Note {
                            visible: Backend.Updater.portable
                            text: qsTr("Versión portable: descarga la nueva versión desde la página de la publicación.")
                            color: Theme.textSecondary
                        }
                        Note { text: qsTr("Datos y reproducción vía Spotify."); color: Theme.textSecondary }
                        Note { text: qsTr("Letras sincronizadas de LRCLIB."); color: Theme.textSecondary }
                        Note { text: qsTr("Audio con librespot (licencia MIT)."); color: Theme.textSecondary }
                        Note { text: qsTr("Inicio y otras funciones adaptadas de Spotifast, de Carmine Paolino (licencia MIT). Gracias."); color: Theme.textSecondary }
                        Note { text: qsTr("Velo es un proyecto independiente y no está afiliado a Spotify.") }
                    }
                    Card {
                        heading: qsTr("Atajos de teclado")
                        Note {
                            color: Theme.textSecondary
                            text: qsTr("Espacio: reproducir o pausar\nCtrl + flecha derecha / izquierda: siguiente / anterior\nCtrl + flecha arriba / abajo: subir / bajar volumen\nMayús + flecha derecha / izquierda: avanzar / retroceder 5 segundos\nCtrl + S: activar o desactivar aleatorio\nCtrl + R: cambiar repetición\nCtrl + L: guardar o quitar de Tus me gusta\nAlt + flecha izquierda / derecha: atrás / adelante\nCtrl + + / − / 0: aumentar / reducir / restablecer tamaño")
                        }
                    }
                }
            }
        }
    }
}
