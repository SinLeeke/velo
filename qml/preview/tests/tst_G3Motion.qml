import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/library"
import "../../components/nowplaying"
import "../../theme"

// G3: motion of the player surfaces. Behavioral checks only (no screenshots): the crossfade
// states on a song change, that Modo rendimiento / Reducir movimiento make them instant with the
// right final state, the wave <-> straight crossfade of the seek bar, and the dimming of the bars.
TestCase {
    id: test
    name: "G3Motion"
    width: 900
    height: 700
    visible: true
    when: windowShown

    Component { id: infoComponent; TrackInfo { width: 600; title: "Uno"; artist: "Artista uno"; album: "Álbum uno" } }
    Component { id: seekComponent; SeekBar { width: 500; durationMs: 100000; positionMs: 50000; playing: true; clockRunning: true; clock: 1 } }
    Component { id: panelComponent; NowPlayingSidePanel { width: Theme.sidePanel.width; height: 700; shown: true } }
    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }

    property string savedTitle
    property url savedCover
    function init() {
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Theme.playing = true
        Theme.windowVisible = true
        Backend.Player.canControl = true
        Backend.Player.djSpeaking = false
        savedTitle = Backend.Player.title
        savedCover = Backend.Player.coverUrl
    }
    function cleanup() {
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Backend.Player.title = savedTitle
        Backend.Player.coverUrl = savedCover
        Backend.Player.djSpeaking = false
        Backend.Player.canControl = true
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }

    // ---- A-N1: TrackInfo crossfades title / artist / album.
    function test_trackinfo_crossfade_runs_and_settles() {
        const info = make(infoComponent)
        const title = findChild(info, "trackInfoTitle")
        compare(title.live.text, "Uno")
        compare(title.live.opacity, 1)
        info.title = "Dos"
        info.artist = "Artista dos"
        info.album = "Álbum dos"
        compare(title.live.text, "Dos")
        compare(title.ghost.text, "Uno")
        compare(title.ghost.opacity, 1)    // old text still fully visible when the fade starts
        compare(title.live.opacity, 0)
        tryCompare(title.ghost, "opacity", 0, 2000)
        tryCompare(title.live, "opacity", 1, 2000)
        const artist = findChild(info, "trackInfoArtist"), album = findChild(info, "trackInfoAlbum")
        compare(artist.live.text, "Artista dos")
        tryCompare(artist.live, "opacity", 1, 2000)
        compare(album.live.text, "Álbum dos")
        tryCompare(album.live, "opacity", 1, 2000)
    }

    function test_trackinfo_is_instant_without_motion() {
        const info = make(infoComponent)
        const title = findChild(info, "trackInfoTitle")
        Theme.performanceMode = true
        compare(Theme.motion.base, 0)
        info.title = "Tres"
        compare(title.live.text, "Tres")
        compare(title.live.opacity, 1)
        compare(title.ghost.opacity, 0)
        Theme.performanceMode = false
        Theme.reduceMotion = true
        info.title = "Cuatro"
        compare(title.live.opacity, 1)
        compare(title.ghost.opacity, 0)
    }

    function test_trackinfo_retarget_ends_on_the_last_text() {
        const info = make(infoComponent)
        const title = findChild(info, "trackInfoTitle")
        info.title = "Dos"
        wait(40)
        info.title = "Tres"       // interrupts the fade: the last text wins, nothing is left half-way
        compare(title.live.text, "Tres")
        tryCompare(title.live, "opacity", 1, 2000)
        tryCompare(title.ghost, "opacity", 0, 2000)
    }

    // ---- A-N2: wave <-> straight crossfade.
    function test_seekbar_wave_and_straight_crossfade() {
        const seek = make(seekComponent)
        const layer = findChild(seek, "seekWaveLayer")
        verify(layer !== null)
        tryCompare(layer, "opacity", 1, 2000)
        verify(layer.visible)
        seek.dragFraction = 0.5
        seek.dragging = true          // the played part turns straight, the wave fades out first
        verify(layer.visible)
        tryCompare(layer, "opacity", 0, 2000)
        tryCompare(layer, "visible", false, 2000)
        seek.dragging = false
        tryCompare(layer, "opacity", 1, 2000)
    }

    function test_seekbar_instant_without_motion() {
        Theme.performanceMode = true
        const seek = make(seekComponent)
        const layer = findChild(seek, "seekWaveLayer")
        compare(seek.waveMoving, false)       // no motion: deliberately straight
        tryCompare(layer, "opacity", 0, 500)
        compare(layer.visible, false)
    }

    // ---- A-N4: the seek bar dims when it is locked.
    function test_seekbar_dims_when_disabled() {
        const seek = make(seekComponent)
        seek.enabled = false
        const track = layerParentTrack(seek)
        tryCompare(track, "opacity", Theme.library.disabledOpacity, 2000)
        seek.enabled = true
        tryCompare(track, "opacity", 1, 2000)
    }
    function layerParentTrack(seek) { return findChild(seek, "seekWaveLayer").parent }

    // ---- A-B1: the library bar's progress slider dims with DJ Livi, with the same lock.
    function test_bar_slider_dims_while_dj_speaks() {
        const bar = make(barComponent)
        const seek = findChild(bar, "playerBarSeek")
        tryCompare(seek, "opacity", 1, 2000)
        Backend.Player.djSpeaking = true
        compare(seek.enabled, false)
        tryCompare(seek, "opacity", Theme.library.disabledOpacity, 2000)
        Backend.Player.djSpeaking = false
        tryCompare(seek, "opacity", 1, 2000)
    }

    function test_bar_slider_dim_is_instant_without_motion() {
        const bar = make(barComponent)
        const seek = findChild(bar, "playerBarSeek")
        Theme.performanceMode = true
        Backend.Player.djSpeaking = true
        tryCompare(seek, "opacity", Theme.library.disabledOpacity, 500)
    }

    // ---- A-S1 / A-S2: side panel.
    function test_panel_text_and_cover_crossfade() {
        const panel = make(panelComponent)
        const title = findChild(panel, "sidePanelTitle")
        const cover = findChild(panel, "sidePanelCover")
        compare(title.live.text, Backend.Player.title)
        const firstFront = cover.front
        Backend.Player.title = "Otra canción"
        Backend.Player.coverUrl = Qt.resolvedUrl("../../mock/assets/cover_green.png")
        compare(title.live.text, "Otra canción")
        compare(title.ghost.text, savedTitle)
        tryCompare(title.live, "opacity", 1, 2000)
        tryCompare(title.ghost, "opacity", 0, 2000)
        tryVerify(() => cover.front !== firstFront, 3000)
        tryCompare(cover.front, "opacity", 1, 2000)
        tryCompare(cover.back, "opacity", 0, 2000)
        compare(cover.front.source, Backend.Player.coverUrl)
    }

    function test_panel_change_is_instant_without_motion() {
        const panel = make(panelComponent)
        const title = findChild(panel, "sidePanelTitle")
        const cover = findChild(panel, "sidePanelCover")
        Theme.performanceMode = true
        Backend.Player.title = "Sin movimiento"
        Backend.Player.coverUrl = Qt.resolvedUrl("../../mock/assets/cover_green.png")
        compare(title.live.text, "Sin movimiento")
        compare(title.live.opacity, 1)
        compare(title.ghost.opacity, 0)
        tryVerify(() => cover.front.source == Backend.Player.coverUrl, 3000)
        tryCompare(cover.front, "opacity", 1, 1000)
        tryCompare(cover.back, "opacity", 0, 1000)
    }

    function test_panel_cards_end_fully_visible() {
        const panel = make(panelComponent)
        wait(400)    // let the panel finish entering: cards then fade in on their own
        Backend.Lyrics.status = "none"
        const lyrics = findChild(panel, "sidePanelLyrics")
        Backend.Lyrics.status = "synced"
        tryCompare(lyrics, "visible", true, 1000)
        tryCompare(lyrics, "opacity", 1, 2000)
        Backend.Lyrics.status = "none"
        tryCompare(lyrics, "visible", false, 1000)
        Backend.Lyrics.status = "synced"
        tryCompare(lyrics, "opacity", 1, 2000)
    }
}
