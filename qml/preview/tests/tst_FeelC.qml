import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/library"
import "../../components/nowplaying"
import "../../components/devices"
import "../../theme"

// Feel C: heart in Now Playing, lyrics button in the bar, side-panel context title, the round
// play disc (glyph swap, no scallop), and the «Conectando…» state of the device picker.
TestCase {
    id: test
    name: "FeelC"
    width: 900
    height: 700
    visible: true
    when: windowShown

    Component { id: infoComponent; TrackInfo { width: 600; title: "Uno"; artist: "Artista uno"; album: "Álbum uno" } }
    Component { id: discComponent; PlayDisc { } }
    Component { id: panelComponent; NowPlayingSidePanel { width: Theme.sidePanel.width; height: 700; shown: true } }
    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }
    Component { id: pickerComponent; DevicePicker {} }

    function init() {
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Theme.playing = true
        Theme.windowVisible = true
        Backend.Player.canControl = true
        Backend.Player.hasSession = true
    }
    function cleanup() {
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Backend.Player.contextTitle = ""
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }

    function test_heart_toggles_like() {
        const info = make(infoComponent)
        const heart = findChild(info, "trackInfoHeart")
        verify(heart !== null)
        Backend.Player.liked = false
        heart.clicked()
        compare(Backend.Player.liked, true)
        heart.clicked()
        compare(Backend.Player.liked, false)
    }
    function test_heart_never_overlaps_more_button() {
        const info = make(infoComponent)
        const heart = findChild(info, "trackInfoHeart")
        verify(heart.x + heart.width <= info.moreButton.x)
    }
    function test_lyrics_button_toggles_setting_and_opens_panel() {
        const bar = make(barComponent)
        const button = findChild(bar, "playerBarLyrics")
        verify(button !== null)
        let opened = 0
        bar.sidePanelToggled.connect(() => opened++)
        Backend.Settings.showLyrics = false
        button.clicked()
        compare(Backend.Settings.showLyrics, true)
        compare(button.active, true)
        compare(opened, 1)
        button.clicked()
        compare(Backend.Settings.showLyrics, false)
        compare(opened, 1)
    }
    function test_side_panel_header_uses_context_title() {
        const panel = make(panelComponent)
        const header = findChild(panel, "sidePanelContext")
        verify(header !== null)
        Backend.Player.contextTitle = "Mi lista"
        wait(0)
        compare(header.text, "Mi lista")
        Backend.Player.contextTitle = ""
        compare(header.text, Backend.Player.album)
    }
    function test_play_disc_glyph_swap_final_state() {
        const disc = make(discComponent)
        const playG = findChild(disc, "playGlyph")
        const pauseG = findChild(disc, "pauseGlyph")
        Theme.reduceMotion = true
        disc.playing = false
        tryCompare(playG, "opacity", 1)
        compare(pauseG.visible, false)
        disc.playing = true
        tryCompare(pauseG, "opacity", 1)
        tryCompare(playG, "opacity", 0)
        compare(playG.visible, false)
    }
    function test_play_disc_halo_off_when_paused_or_performance() {
        const disc = make(discComponent)
        disc.playing = true
        Theme.playing = true
        compare(disc.haloOn, true)
        disc.playing = false
        compare(disc.haloOn, false)
        disc.playing = true
        Theme.performanceMode = true
        compare(disc.haloOn, false)
    }
    function test_device_picker_pending_until_active() {
        Backend.Player.devices.clear()
        Backend.Player.devices.append({deviceId: "dA", name: "Uno", kind: "Computer", isActive: true})
        Backend.Player.devices.append({deviceId: "dB", name: "Dos", kind: "Smartphone", isActive: false})
        const picker = createTemporaryObject(pickerComponent, test)
        picker.open()
        tryCompare(picker, "opened", true)
        picker.pendingId = "dB"
        compare(picker.pendingId, "dB")
        Backend.Player.devices.setProperty(0, "isActive", false)
        Backend.Player.devices.setProperty(1, "isActive", true)
        tryCompare(picker, "opened", false)
        tryCompare(picker, "pendingId", "")
    }
}
