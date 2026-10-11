import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views

// T4: lyrics with another device playing. The backend feeds the same position channel; the UI
// shows the lyrics (and their button) while that progress is usable and hides them, instead of an
// empty panel, when the remote reports no duration.
TestCase {
    id: test
    name: "T4Lyrics"
    when: windowShown
    visible: true
    width: 1300
    height: 800
    Component { id: npComponent; Views.NowPlaying { width: test.width; height: test.height } }
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }
    property int duration: 0

    function init() {
        Theme.reduceMotion = true
        duration = Backend.Player.durationMs
        Backend.Account.signedIn = true
        Backend.Player.hasSession = true
        Backend.Settings.showLyrics = true
        Backend.Lyrics.status = "synced"
        Backend.Player.remote = false
    }
    function cleanup() { Backend.Player.remote = false; Backend.Player.durationMs = duration }

    function test_remote_with_progress_keeps_the_lyrics() {
        const np = createTemporaryObject(npComponent, test)
        const button = findChild(np, "npLyricsButton")
        Backend.Player.remote = true
        compare(np.lyricsReliable, true)
        compare(np.lyricsShown, true)
        compare(button.visible, true)
    }

    function test_remote_without_progress_hides_the_button_and_the_panel() {
        const np = createTemporaryObject(npComponent, test)
        const button = findChild(np, "npLyricsButton")
        Backend.Player.remote = true
        Backend.Player.durationMs = 0
        compare(np.lyricsReliable, false)
        compare(np.lyricsShown, false)
        compare(button.visible, false)
        // The rest of the pill stays reachable.
        compare(findChild(np, "npQueueButton").visible, true)
        Backend.Player.remote = false
        compare(button.visible, true)
    }

    function test_side_panel_card_follows_the_same_rule() {
        const lib = createTemporaryObject(libraryComponent, test)
        lib.sidePanelWanted = true
        const card = findChild(lib.sidePanel, "sidePanelLyrics")
        compare(card.visible, true)
        Backend.Player.remote = true
        Backend.Player.durationMs = 0
        compare(card.visible, false)
        Backend.Player.durationMs = 200000
        compare(card.visible, true)
    }
}
