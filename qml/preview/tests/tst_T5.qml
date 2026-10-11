import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views

// T5: «Reproduciendo» side panel of the library: on by default, the bar button toggles it, it
// reserves its column (rightInset), the queue replaces it, a narrow window hides it, lyrics /
// credits / next in the queue show only with data.
TestCase {
    id: test
    name: "T5"
    when: windowShown
    visible: true
    width: 1500
    height: 800
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }

    function init() {
        Theme.reduceMotion = true
        Backend.Account.signedIn = true
        Backend.Account.audioReady = true
        Backend.Player.hasSession = true
        Backend.Lyrics.status = "synced"
    }
    function make() {
        const lib = createTemporaryObject(libraryComponent, test)
        verify(lib !== null)
        lib.sidePanelWanted = true
        return lib
    }

    function test_on_by_default_and_reserves_its_column() {
        const lib = make()
        compare(lib.sidePanelShown, true)
        compare(lib.sidePanel.shown, true)
        compare(lib.sidePanelInset, Theme.sidePanel.width)
        compare(lib.rightInset, Theme.sidePanel.width)
        tryVerify(() => lib.sidePanel.visible)
        compare(findChild(lib.sidePanel, "sidePanelTitle").text, Backend.Player.title)
    }

    function test_bar_button_toggles_and_shows_state() {
        const lib = make()
        const button = findChild(lib, "playerBarSidePanel")
        verify(button !== null)
        compare(button.active, true)
        compare(button.toggle, true)
        verify(button.text !== "")
        mouseClick(button)
        compare(lib.sidePanelWanted, false)
        compare(button.active, false)
        compare(lib.rightInset, 0)
        tryCompare(lib.sidePanel, "visible", false)
        mouseClick(button)
        compare(lib.sidePanelWanted, true)
        compare(lib.rightInset, Theme.sidePanel.width)
    }

    function test_queue_replaces_the_panel() {
        const lib = make()
        compare(lib.sidePanelShown, true)
        lib.queueInset = 340
        compare(lib.sidePanelShown, false)
        compare(lib.rightInset, 340)         // the queue's width only, never both
        lib.queueInset = 0
        compare(lib.sidePanelShown, true)
    }

    function test_narrow_window_hides_it() {
        const lib = createTemporaryObject(libraryComponent, test, {width: 760})
        lib.sidePanelWanted = true
        compare(lib.sidePanelFits, false)
        compare(lib.sidePanelShown, false)
        compare(lib.rightInset, 0)
        lib.width = test.width
        compare(lib.sidePanelShown, true)
    }

    function test_nothing_playing_hides_it() {
        const lib = make()
        Backend.Player.hasSession = false
        compare(lib.sidePanelShown, false)
        Backend.Player.hasSession = true
    }

    function test_lyrics_card_only_with_lyrics() {
        const lib = make()
        const card = findChild(lib.sidePanel, "sidePanelLyrics")
        compare(card.visible, true)
        Backend.Lyrics.status = "none"
        compare(card.visible, false)
        Backend.Lyrics.status = "loading"
        compare(card.visible, false)
        Backend.Lyrics.status = "plain"
        compare(card.visible, true)
        Backend.Lyrics.status = "synced"
    }

    function test_credits_and_next_in_queue() {
        const lib = make()
        const credits = findChild(lib.sidePanel, "sidePanelCredits")
        tryCompare(credits, "visible", true)           // the mock answers at once
        const next = findChild(lib.sidePanel, "sidePanelNext")
        compare(next.visible, Backend.Player.queue.count > 0)
        compare(findChild(lib.sidePanel, "sidePanelNextTitle").text, Backend.Player.queue.get(0).title)
        let asked = 0
        lib.queueRequested.connect(() => asked++)
        findChild(lib.sidePanel, "sidePanelOpenQueue").clicked()   // may sit below the fold of the scroll
        compare(asked, 1)
    }
}
