import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/library"
import "../../components/nowplaying"
import "../../theme"

// Mixear for what is playing: the toggle in the player bar and in Reproduciendo ahora appears only
// for playlists / Tus me gusta while Velo's own engine can mix (gapless on, no DJ), and flips the
// playing context's mark.
TestCase {
    id: test
    name: "MixPlaying"
    width: 900
    height: 700
    visible: true
    when: windowShown

    Component { id: infoComponent; TrackInfo { width: 600; title: "Uno"; artist: "Artista uno"; album: "Álbum uno" } }
    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }

    function init() {
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Theme.playing = true
        Theme.windowVisible = true
        Backend.Player.canControl = true
        Backend.Player.hasSession = true
        Backend.Settings.gapless = true
        Backend.Player.contextUri = "spotify:playlist:abc"
        Backend.Library.mixPlaying = false
        Backend.Library.mixPlayingToggles = 0
    }
    function cleanup() {
        Backend.Player.contextUri = ""
        Backend.Settings.gapless = true
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }

    function test_visible_for_playlist_and_liked_only_with_gapless() {
        const bar = make(barComponent)
        const mix = findChild(bar, "playerBarMix")
        verify(mix !== null)
        compare(mix.visible, true)
        Backend.Player.contextUri = "spotify:user:yo:collection"
        compare(mix.visible, true)
        Backend.Player.contextUri = "spotify:album:xyz"
        compare(mix.visible, false)
        Backend.Player.contextUri = ""
        compare(mix.visible, false)
        Backend.Player.contextUri = "spotify:playlist:abc"
        Backend.Settings.gapless = false
        compare(mix.visible, false)
    }
    function test_bar_button_toggles_playing_context_mark() {
        const bar = make(barComponent)
        const mix = findChild(bar, "playerBarMix")
        mix.clicked()
        compare(Backend.Library.mixPlayingToggles, 1)
        compare(Backend.Library.mixPlaying, true)
        compare(mix.active, true)
    }
    function test_now_playing_button_follows_context_and_toggles() {
        const info = make(infoComponent)
        const mix = findChild(info, "trackInfoMix")
        verify(mix !== null)
        compare(mix.visible, true)
        verify(mix.x + mix.width <= info.moreButton.x)
        mix.clicked()
        compare(Backend.Library.mixPlayingToggles, 1)
        Backend.Player.contextUri = "spotify:album:xyz"
        compare(mix.visible, false)
    }
}
