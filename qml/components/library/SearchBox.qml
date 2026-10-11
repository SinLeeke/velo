import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import QtQuick.Effects
import Velo.Backend as Backend
import "../../theme"
import ".."
import "../motion"

// T3: the search box of the library's top bar, like Spotify. A pill centered in the title strip
// (Library.qml sizes and places it); typing drops the closest matches under it, over whatever
// page is open, without navigating:
//  - Typing asks Rust for suggestions (Library.suggestSearch: 250 ms debounce and cancellation in
//    Tokio, so there is no QML Timer). ↑/↓ move, Enter opens the highlighted row, Enter with no
//    row highlighted emits `submitted` (Library opens the detailed results page), Esc closes the
//    list and a second Esc leaves the field.
//  - A suggested artist, album or playlist emits `navigate`; a song runs the full search for it
//    (so "Mejor resultado" shows it with its play button), as the search page did before T3.
// Spotify gives no text autocomplete: suggestions are real catalogue rows (docs/F1-F2-busqueda.md).
// There is no search history in the backend yet, so an empty focused field shows nothing.
Item {
    id: box
    property alias text: field.text
    readonly property alias field: field
    readonly property alias suggestionPopup: suggestions
    property int suggestIndex: -1
    signal submitted(string query)
    signal navigate(string kind, string itemId, string owner)

    implicitHeight: Theme.searchBox.height

    function kindLabel(k) {
        switch (k) {
        case "track": return qsTr("Canción")
        case "artist": return qsTr("Artista")
        case "album": return qsTr("Álbum")
        case "playlist": return qsTr("Playlist")
        default: return ""
        }
    }
    // Ctrl+F / Ctrl+L / «Buscar» of the sidebar: into the field with its text selected.
    function focusField() {
        field.forceActiveFocus(Qt.ShortcutFocusReason)
        field.selectAll()
    }
    function closeSuggestions() {
        dismissed = true
        Backend.Library.cancelSearchSuggestions()
    }
    function submit(text) {
        const q = (text === undefined ? field.text : text).trim()
        if (q === "") return
        if (field.text !== q) field.text = q
        closeSuggestions()
        submitted(q)
    }
    function openRow(row) {
        closeSuggestions()
        if (row.kind === "track") submit(row.subtitle !== "" ? row.title + " " + row.subtitle : row.title)
        else navigate(row.kind, row.objectId, row.kind === "playlist" ? row.subtitle : "")
    }

    // R4 (P3-6): saved state of the suggested songs. Asked from Spotify when a suggestion is
    // hovered or highlighted (Actions.inspectSaved) and kept from the saves made here.
    property var savedSongs: ({})
    function noteSaved(uri, saved) {
        const next = Object.assign({}, savedSongs)
        next[uri] = saved
        savedSongs = next
    }
    Connections {
        target: Backend.Actions
        function onSavedChanged(uri, saved) { if (uri.startsWith("spotify:track:")) box.noteSaved(uri, saved) }
        function onInspectedSavedChanged() {
            const uri = Backend.Actions.inspectedUri
            if (uri.startsWith("spotify:track:") && Backend.Actions.inspectedSaved >= 0)
                box.noteSaved(uri, Backend.Actions.inspectedSaved === 1)
        }
    }

    LibraryField {
        id: field
        objectName: "librarySearchField"
        anchors.fill: parent
        leftPadding: Theme.searchBox.iconInset + Theme.icon.md + Theme.space.sm
        rightPadding: clearButton.visible ? clearButton.width + Theme.space.xs
                    : hint.visible ? hint.width + Theme.space.md : Theme.space.md
        topPadding: 0
        bottomPadding: 0
        verticalAlignment: TextInput.AlignVCenter
        placeholderText: qsTr("¿Qué quieres reproducir?")
        accessibleName: qsTr("Buscar")
        // A search box with a list of suggestions under it: a combobox for Narrator.
        Accessible.role: Accessible.ComboBox
        Accessible.description: qsTr("¿Qué quieres reproducir? Atajo: Ctrl+L")
        background: Rectangle {
            radius: height / 2
            color: field.hovered && !field.activeFocus ? Theme.searchBox.fillHover : Theme.searchBox.fill
            // controlBorder keeps the pill findable (>= 3:1); the focus ring when focused.
            border.color: field.activeFocus ? Theme.focusRing : field.hovered ? Theme.textSecondary : Theme.controlBorder
            border.width: field.activeFocus ? Theme.stroke.focus : Theme.stroke.hairline
        }
        IconImage {
            x: Theme.searchBox.iconInset
            anchors.verticalCenter: parent.verticalCenter
            source: field.activeFocus ? Qt.resolvedUrl("../../icons/search_fill.svg") : Qt.resolvedUrl("../../icons/search.svg")
            sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
            color: field.activeFocus ? Theme.text : Theme.textSecondary
        }
        // The shortcut, like Spotify's chip; only at rest and where it fits.
        Rectangle {
            id: hint
            anchors.right: parent.right
            anchors.rightMargin: Theme.space.md
            anchors.verticalCenter: parent.verticalCenter
            visible: field.length === 0 && !field.activeFocus && box.width >= Theme.searchBox.hintMinWidth
            width: hintLabel.implicitWidth + 2 * Theme.searchBox.hintPadding
            height: hintLabel.implicitHeight + Theme.searchBox.hintPadding
            radius: Theme.radius.xs
            color: Theme.library.clearPaint
            border.width: Theme.stroke.hairline
            border.color: Theme.hairline
            Accessible.ignored: true
            LibraryLabel {
                id: hintLabel
                anchors.centerIn: parent
                text: qsTr("Ctrl+L")
                color: Theme.textSecondary
                font.pixelSize: Theme.type.caption
            }
        }
        IconButton {
            id: clearButton
            objectName: "searchClearButton"
            anchors.right: parent.right
            anchors.rightMargin: Theme.space.xxs
            anchors.verticalCenter: parent.verticalCenter
            visible: field.length > 0
            focusPolicy: Qt.NoFocus      // Esc clears the list from the keyboard; Tab skips it
            text: qsTr("Borrar búsqueda")
            iconSource: Qt.resolvedUrl("../../icons/close.svg")
            iconSize: Theme.icon.sm
            ink: Theme.textSecondary
            onClicked: {
                field.clear()
                box.closeSuggestions()
                field.forceActiveFocus(Qt.MouseFocusReason)
            }
        }
        onTextEdited: {
            box.suggestIndex = -1
            box.dismissed = false
            Backend.Library.suggestSearch(text)
        }
        onActiveFocusChanged: { if (!activeFocus) Backend.Library.cancelSearchSuggestions() }
        Keys.onDownPressed: {
            if (!suggestions.opened && Backend.Library.searchSuggestions.count > 0) box.dismissed = false
            box.suggestIndex = Math.min(suggestionList.count - 1, box.suggestIndex + 1)
        }
        Keys.onUpPressed: box.suggestIndex = Math.max(-1, box.suggestIndex - 1)
        // First Esc closes the list, the second one leaves the field (focus back to the window).
        // The field claims Esc ahead of any window Shortcut while it has the focus.
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Escape) event.accepted = true }
        Keys.onEscapePressed: event => {
            if (suggestions.opened) box.closeSuggestions()
            else field.focus = false
            event.accepted = true
        }
        onAccepted: {
            const row = suggestions.opened && box.suggestIndex >= 0 ? suggestionList.itemAtIndex(box.suggestIndex) : null
            if (row) box.openRow(row)
            else box.submit()
        }
    }

    // U4: the popup follows one condition instead of opening on a query change. It opens when
    // rows (or an error) are there for a non-empty field with focus, and closes when the field
    // empties, loses focus, Esc dismisses it or a press lands outside. Rust keeps the previous
    // rows until the new query answers, so typing does not blink it. Binding `visible` on a Popup
    // opens/closes it for good, so open()/close() it is. It never takes focus: the field keeps
    // typing. The model's own `count` is read: the ListView inside the closed popup does not apply
    // row changes until it is shown. `dismissed` (Esc, a press outside) lasts until the next edit.
    property bool dismissed: false
    readonly property bool suggestionsWanted: field.activeFocus && field.text.trim() !== "" && !dismissed
        && Backend.Library.searchSuggestionQuery !== ""
        && (Backend.Library.searchSuggestions.count > 0 || Backend.Library.searchSuggestionError !== "")
    onSuggestionsWantedChanged: {
        if (suggestionsWanted) { if (!suggestions.opened) suggestions.open() }
        else if (suggestions.opened) suggestions.close()
    }
    Popup {
        id: suggestions
        // Under the pill, centered on it; never narrower than a readable list, and kept inside the
        // window (margins) when the pill sits near an edge.
        width: Math.max(box.width, Theme.searchBox.popupMinWidth)
        x: Math.round((box.width - width) / 2)
        y: box.height + Theme.space.xs
        margins: Theme.searchBox.popupMargin
        padding: Theme.space.xs
        focus: false
        modal: false
        // Esc is the field's (first closes, second leaves); a press outside the box closes it.
        closePolicy: Popup.CloseOnPressOutsideParent
        // Motion (S1-2): only the opening animates (fade + scale 0.98 -> 1 from the field above,
        // `instant`); while typing the rows change instantly and closing is immediate.
        transformOrigin: Popup.Top
        enter: null
        property PopupEntrance entrance: PopupEntrance { popup: suggestions; fromScale: Theme.entrance.menuScale; duration: Theme.motion.instant }
        exit: null
        onClosed: {
            box.suggestIndex = -1
            if (box.suggestionsWanted) box.dismissed = true   // closed from outside (a press elsewhere)
        }
        // P2-1: lifted off the page like the menus (one RectangularShadow, no layer).
        background: Rectangle {
            color: Theme.surface2
            radius: Theme.radius.lg
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
        contentItem: Column {
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Sugerencias de búsqueda")
            ListView {
                id: suggestionList
                objectName: "searchSuggestionList"
                width: parent.width
                height: count * Theme.size.trackRow
                interactive: false
                reuseItems: true
                currentIndex: box.suggestIndex
                model: Backend.Library.searchSuggestions
                delegate: ItemDelegate {
                    id: suggestion
                    required property int index
                    required property string kind
                    required property string objectId
                    required property string uri
                    required property string title
                    required property string subtitle
                    required property string imageUrl
                    readonly property bool current: index === box.suggestIndex
                    readonly property bool isSong: kind === "track"
                    readonly property bool saved: isSong && box.savedSongs[uri] === true
                    readonly property bool savedKnown: isSong && box.savedSongs[uri] !== undefined
                    // Ask once the suggestion is pointed at (mouse or arrows), not for every row.
                    readonly property bool pointed: current || hovered
                    onPointedChanged: if (pointed && isSong && box.savedSongs[uri] === undefined) Backend.Actions.inspectSaved(uri)
                    width: suggestionList.width
                    height: Theme.size.trackRow
                    padding: Theme.space.sm
                    hoverEnabled: true
                    focusPolicy: Qt.NoFocus
                    Accessible.role: Accessible.ListItem
                    Accessible.name: qsTr("%1, %2").arg(title).arg(box.kindLabel(kind))
                    Accessible.selected: current
                    background: Rectangle {
                        radius: Theme.radius.md
                        color: suggestion.current || suggestion.hovered ? Theme.surface3 : Theme.library.clearPaint
                    }
                    contentItem: Row {
                        spacing: Theme.space.md
                        LibraryArtwork {
                            width: Theme.size.trackRow - 2 * Theme.space.sm
                            height: width
                            anchors.verticalCenter: parent.verticalCenter
                            source: suggestion.imageUrl
                            portrait: suggestion.kind === "artist"
                            fallback: suggestion.kind === "artist" ? Qt.resolvedUrl("../../icons/person.svg")
                                    : suggestion.kind === "track" ? Qt.resolvedUrl("../../icons/music_note.svg") : Qt.resolvedUrl("../../icons/library_music.svg")
                        }
                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            width: suggestion.width - 2 * suggestion.padding - Theme.size.trackRow - Theme.space.md
                                   - (addButton.visible ? Theme.size.iconButton + Theme.space.md : 0)
                            spacing: Theme.space.xxs
                            LibraryLabel { width: parent.width; text: suggestion.title; font.weight: Theme.type.semibold }
                            LibraryLabel {
                                width: parent.width
                                text: suggestion.subtitle !== "" ? qsTr("%1 · %2").arg(box.kindLabel(suggestion.kind)).arg(suggestion.subtitle) : box.kindLabel(suggestion.kind)
                                color: Theme.textSecondary
                                font.pixelSize: Theme.type.caption
                            }
                        }
                        // P3-6: the heart of the rows, with its state: saves or removes the song
                        // and pops when it is saved.
                        IconButton {
                            id: addButton
                            objectName: "suggestionSaveButton"
                            anchors.verticalCenter: parent.verticalCenter
                            visible: suggestion.isSong
                            focusPolicy: Qt.NoFocus
                            enabled: suggestion.savedKnown
                            Accessible.checkable: true
                            text: !suggestion.savedKnown ? qsTr("Comprobando Tus me gusta…") : suggestion.saved ? qsTr("Quitar de Tus me gusta") : qsTr("Guardar en Tus me gusta")
                            iconSource: suggestion.saved ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : Qt.resolvedUrl("../../icons/favorite.svg")
                            iconSize: Theme.icon.sm
                            ink: Theme.textSecondary
                            active: suggestion.saved
                            activeFill: Theme.library.clearPaint
                            activeInk: Theme.accent
                            onClicked: Backend.Actions.setSaved(suggestion.uri, !suggestion.saved)
                            Connections {
                                target: Backend.Actions
                                function onSavedChanged(uri, saved) { if (saved && uri === suggestion.uri) addButton.pop() }
                            }
                        }
                    }
                    onClicked: box.openRow(suggestion)
                }
            }
            LibraryLabel {
                visible: suggestionList.count === 0 && Backend.Library.searchSuggestionError !== ""
                width: parent.width
                padding: Theme.space.md
                text: Backend.Library.searchSuggestionError
                color: Theme.textSecondary
                wrapMode: Text.Wrap
            }
        }
    }
}
