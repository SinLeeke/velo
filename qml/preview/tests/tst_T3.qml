import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views

// T3: the search box lives in the library's top bar; typing drops suggestions over the current
// page without navigating; Enter with nothing highlighted opens the results page; Esc closes the
// list, a second Esc leaves the field; «Buscar» of the sidebar focuses the field.
TestCase {
    id: test
    name: "T3"
    when: windowShown
    visible: true
    width: 1100
    height: 760
    Item { id: elsewhere; width: 20; height: 20; focus: true }
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }
    property string controlsBefore: ""

    function initTestCase() { controlsBefore = Backend.Settings.windowControls }
    function init() {
        Theme.reduceMotion = true
        Backend.Account.signedIn = true
        Backend.Account.audioReady = true
        Backend.Library.searchQuery = ""
        Backend.Library.searchError = ""
        Backend.Library.searchLoading = false
        Backend.Library.cancelSearchSuggestions()
        elsewhere.forceActiveFocus()
    }
    function cleanup() {
        Backend.Settings.windowControls = controlsBefore
        Backend.Library.searchQuery = ""
        Backend.Library.cancelSearchSuggestions()
        Backend.Library.setSearchType("all")
    }
    function makeLibrary() {
        const library = createTemporaryObject(libraryComponent, test)
        verify(library !== null)
        compare(library.pageKind, "home")
        return library
    }
    function typeText(text) {
        for (let i = 0; i < text.length; ++i) keyClick(text[i])
    }

    function test_field_sits_in_the_top_bar() {
        const library = makeLibrary()
        const box = library.searchBox
        const field = box.field
        verify(field.visible)
        compare(field.objectName, "librarySearchField")
        compare(field.Accessible.role, Accessible.ComboBox)
        verify(field.Accessible.name !== "")
        // Inside the title strip, centered over the page column when there is room.
        verify(box.y >= 0 && box.y + box.height <= Theme.size.titleBar, "inside the title strip")
        compare(box.width, Theme.searchBox.maxWidth)
        verify(Math.abs(box.x + box.width / 2 - (library.sidebarWidth + library.width) / 2) <= 1, "centered over the page")
        const settings = findChild(library, "openSettingsButton")
        verify(box.x + box.width <= settings.x, "clear of the buttons at the right")
        verify(box.x >= library.searchLeftWithCrumb - Theme.searchBox.gap, "clear of the crumb")
    }

    function test_never_overlaps_at_minimum_width() {
        Backend.Settings.windowControls = "windows"
        const library = makeLibrary()
        library.width = Theme.size.windowMinWidth
        library.sidebarDragWidth = Theme.size.sidebarMax     // widest sidebar the page allows
        wait(0)
        const box = library.searchBox
        const settings = findChild(library, "openSettingsButton")
        verify(box.width >= Theme.size.iconButton, "still a usable target")
        verify(box.width <= Theme.searchBox.maxWidth)
        verify(box.x + box.width <= settings.x, "never under the settings button")
        verify(box.x >= library.searchLeftBare - Theme.searchBox.gap, "never under Atrás / Adelante")
        const crumb = findChild(library, "libraryCrumb")
        if (crumb.visible) verify(box.x >= crumb.x + crumb.width, "never under the crumb")
        library.sidebarDragWidth = -1
    }

    function test_typing_opens_suggestions_without_navigating() {
        const library = makeLibrary()
        const kindBefore = Backend.Library.currentKind
        const pageBefore = library.currentPageItem
        mouseClick(library.searchBox.field)
        verify(library.searchBox.field.activeFocus)
        typeText("mac")
        compare(library.searchBox.text, "mac")
        tryCompare(library.searchBox.suggestionPopup, "opened", true)
        verify(library.searchBox.field.activeFocus, "the field keeps the focus")
        compare(Backend.Library.currentKind, kindBefore, "no backend navigation")
        compare(library.pageKind, "home")
        compare(library.currentPageItem, pageBefore)
    }

    function test_enter_without_selection_opens_results() {
        const library = makeLibrary()
        library.searchBox.focusField()
        typeText("mac")
        tryCompare(library.searchBox.suggestionPopup, "opened", true)
        keyClick(Qt.Key_Return)
        compare(library.pageKind, "search")
        compare(Backend.Library.searchQuery, "mac")
        compare(library.searchQuery, "mac")
        compare(library.searchBox.text, "mac", "the text stays in the field")
        tryCompare(library.searchBox.suggestionPopup, "opened", false)
        verify(library.currentPageItem.hasResults)
        // A second query from the results page replaces it in place.
        const entries = library.history.length
        library.searchBox.focusField()
        typeText("lumen")
        keyClick(Qt.Key_Return)
        compare(Backend.Library.searchQuery, "lumen")
        compare(library.history.length, entries)
        compare(library.history[library.historyIndex].query, "lumen")
    }

    function test_enter_on_highlighted_suggestion_opens_it() {
        const library = makeLibrary()
        library.searchBox.focusField()
        typeText("lu")
        const popup = library.searchBox.suggestionPopup
        tryCompare(popup, "opened", true)
        const list = findChild(popup.contentItem, "searchSuggestionList")
        list.forceLayout()
        tryVerify(() => list.itemAtIndex(1) !== null)
        keyClick(Qt.Key_Down)
        keyClick(Qt.Key_Down)
        compare(library.searchBox.suggestIndex, 1)
        keyClick(Qt.Key_Return)
        compare(library.pageKind, "artist")
        compare(library.itemId, "a1")
        tryCompare(popup, "opened", false)
    }

    function test_escape_closes_then_leaves() {
        const library = makeLibrary()
        const field = library.searchBox.field
        library.searchBox.focusField()
        typeText("mac")
        const popup = library.searchBox.suggestionPopup
        tryCompare(popup, "opened", true)
        keyClick(Qt.Key_Escape)
        tryCompare(popup, "opened", false)
        verify(field.activeFocus, "first Esc keeps the field")
        compare(library.pageKind, "home")
        keyClick(Qt.Key_Escape)
        verify(!field.activeFocus, "second Esc leaves the field")
        compare(field.text, "mac")
        // Typing again brings the list back.
        library.searchBox.focusField()
        keyClick(Qt.Key_End)
        typeText("s")
        tryCompare(popup, "opened", true)
    }

    function test_sidebar_search_focuses_the_field() {
        const library = makeLibrary()
        const field = library.searchBox.field
        library.searchBox.text = "lumen"
        verify(!field.activeFocus)
        const button = findChild(library, "sidebarSearchButton")
        verify(button !== null)
        mouseClick(button)
        verify(field.activeFocus, "Buscar focuses the top bar's field")
        compare(field.selectedText, "lumen", "with its text selected")
        compare(library.pageKind, "home", "no empty search page")
        // Ctrl+F / Ctrl+L go through the same function.
        elsewhere.forceActiveFocus()
        library.focusSearch()
        verify(field.activeFocus)
    }
}
