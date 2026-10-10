pragma Singleton
import QtQuick

// M1: app-wide requests that any menu can raise without threading signals through every page.
// Main.qml listens to `show` and opens that page in the library (from Now Playing too).
Item {
    id: nav
    signal show(string kind, string itemId)
    // E6: "Ver créditos" from any song menu; the visible view's CreditsDialog answers.
    signal creditsRequested(string uri)
    function showCredits(uri) { creditsRequested(uri) }

    // Clipboard without a backend call: a never-shown TextEdit copies its own selection.
    function copy(text) {
        clip.text = text
        clip.selectAll()
        clip.copy()
        clip.text = ""
    }
    TextEdit { id: clip; visible: false }

    // R1: does the item with keyboard focus use the key itself? Window shortcuts that take a
    // plain key (Space: play/pause) stay off while it does, so Space activates the focused
    // button, switch, chip, row or lyric line, and types in fields, as Windows expects.
    //  - text inputs (they type the key; Ctrl+V pastes there, not as a link);
    //  - anything reachable with Tab (activeFocusOnTab: Controls with a Tab focus policy,
    //    focusable rows and lines);
    //  - a focused list or grid (it moves its current item and activates it with the keys).
    function isTextInput(item) {
        return item !== null && item !== undefined && (item instanceof TextInput || item instanceof TextEdit)
    }
    function focusUsesKeys(item) {
        if (!item)
            return false
        if (isTextInput(item) || item.activeFocusOnTab === true)
            return true
        return item instanceof ListView || item instanceof GridView
    }
}
