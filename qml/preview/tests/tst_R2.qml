import QtQuick
import QtTest
import Velo.Backend
import "../../views" as Views
import "../../components/nowplaying"
import "../../components/queue"
import "../../theme"
import "../../components/nowplaying/vizshape.js" as Viz

// State/keyboard checks only: no screenshots, network or Rust playback.
TestCase {
    id: test
    name: "R2"
    visible: true
    width: 1100
    height: 680
    when: windowShown
    property int closed: 0
    readonly property url teal: Qt.resolvedUrl("../../mock/assets/cover_teal.png")
    readonly property url green: Qt.resolvedUrl("../../mock/assets/cover_green.png")
    property var savedQueue: []
    property string savedTitle
    property string savedUri

    Shortcut { sequence: "Space"; onActivated: Player.playPause() }
    Component {
        id: npComponent
        Views.NowPlaying {
            width: test.width
            height: 560
            ownsCredits: false
            onCloseRequested: test.closed++
        }
    }
    Component { id: artComponent; CoverArt { width: 160; height: 160; tiltEnabled: false } }
    Component { id: backgroundComponent; CoverBackground { width: 320; height: 240; playing: Player.isPlaying } }
    Component { id: queueComponent; QueuePanel {} }
    Component { id: lyricsComponent; LyricsView { width: 400; height: 320; playing: Player.isPlaying } }
    Component { id: volumeComponent; VolumeSlider { width: 320 } }
    Component { id: bandComponent; VisualizerBand { width: 320; height: 80 } }
    Component { id: seekComponent; SeekBar { width: 320; durationMs: 180000; positionMs: 60000; playing: Player.isPlaying } }
    ListModel {
        id: lines
        ListElement { text: "Primera línea"; timeMs: 1000 }
        ListElement { text: "Segunda línea"; timeMs: 2000 }
        ListElement { text: "Tercera línea"; timeMs: 3000 }
    }

    function initTestCase() {
        Qt.application.organizationName = "VeloR2Tests"
        Qt.application.organizationDomain = "velo.test"
        savedTitle = Player.title
        savedUri = Player.trackUri
        for (let i = 0; i < Player.queue.count; ++i)
            savedQueue.push(JSON.parse(JSON.stringify(Player.queue.get(i))))
    }
    function init() {
        Theme.mode = "dark"
        Theme.zoom = 1
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Theme.pauseInBackground = false
        Theme.drift = false
        Settings.drift = false
        Settings.coverBlur = true
        Settings.tintUi = true
        Settings.showLyrics = true
        Settings.performanceKeepVisualizer = false
        Player.hasSession = true
        Player.canControl = true
        Player.queueEditable = true
        Player.queueStatus = "ready"
        Player.isPlaying = true
        Player.volumeRequests = []
        Player.volume = 50
        Player.positionMs = 0
        Player.queueSkipIndex = -1
        Visualizer.enabled = true
        Visualizer.style = "normal"
        Visualizer.running = false
        closed = 0
    }
    function cleanupTestCase() {
        Player.queue.clear()
        for (const row of savedQueue) Player.queue.append(row)
        Player.title = savedTitle
        Player.trackUri = savedUri
        Player.isPlaying = true
        Theme.zoom = 1
        Theme.performanceMode = false
        Settings.drift = true
        Theme.drift = true
    }
    function make(component, properties) {
        const item = createTemporaryObject(component, test, properties || {})
        verify(item !== null)
        return item
    }
    function fillQueue() {
        Player.queue.clear()
        for (let i = 0; i < 4; ++i)
            Player.queue.append({trackId: "r2-" + i, uri: "spotify:track:r2-" + i,
                title: "Pista " + i, artist: "Artista", album: "Álbum", durationMs: 180000,
                imageUrl: "", liked: false, artistId: "", albumId: ""})
    }
    function test_escape_returns_to_library() {
        const np = make(npComponent)
        np.forceActiveFocus()
        keyClick(Qt.Key_Escape)
        compare(closed, 1)
    }
    function test_tab_reaches_top_bar_first() {
        const np = make(npComponent)
        np.forceActiveFocus()
        keyClick(Qt.Key_Tab)
        verify(findChild(np, "npCloseButton").activeFocus)
        keyClick(Qt.Key_Tab)
        verify(findChild(np, "npDevicesButton").activeFocus)
        keyClick(Qt.Key_Tab)
        verify(findChild(np, "npLyricsButton").activeFocus)
        keyClick(Qt.Key_Tab)
        verify(findChild(np, "npQueueButton").activeFocus)
    }
    function test_queue_enter_and_escape() {
        fillQueue()
        const queue = make(queueComponent)
        queue.open()
        const list = findChild(queue, "queueList")
        list.currentIndex = 1
        list.positionViewAtIndex(1, ListView.Contain)
        tryVerify(function () { return list.currentItem !== null })
        list.forceActiveFocus()
        keyClick(Qt.Key_Return)
        compare(Player.queueSkipIndex, 1)
        compare(Player.trackUri, "spotify:track:r2-1")
        keyClick(Qt.Key_Escape)
        compare(queue.opened, false)
        compare(closed, 0)
    }
    function test_queue_click_and_remote_disabled() {
        fillQueue()
        const queue = make(queueComponent)
        queue.open()
        const list = findChild(queue, "queueList")
        list.currentIndex = 0
        list.positionViewAtIndex(0, ListView.Contain)
        tryVerify(function () { return list.currentItem !== null })
        waitForRendering(queue)
        wait(Theme.motion.slow)
        mouseClick(list.currentItem, list.currentItem.width / 2, list.currentItem.height / 2)
        compare(Player.queueSkipIndex, 0)
        Player.queueEditable = false
        list.currentIndex = 0
        list.forceActiveFocus()
        Player.queueSkipIndex = -1
        keyClick(Qt.Key_Return)
        compare(Player.queueSkipIndex, -1)
    }
    function test_queue_tool_is_active() {
        const np = make(npComponent, {queueInset: 300})
        compare(findChild(np, "npQueueButton").active, true)
        np.queueInset = 0
        compare(findChild(np, "npQueueButton").active, false)
        verify(np.activeFocus)
    }
    function test_crossfade_keeps_ready_art_through_empty() {
        const art = make(artComponent, {source: teal})
        tryCompare(art, "hasArt", true)
        const old = art.front
        art.source = ""
        wait(1300) // longer than the abandoned empty-hold timeout
        compare(art.front, old)
        compare(art.hasArt, true)
        art.source = green
        tryVerify(function () { return art.front !== old })
        compare(art.front.status, Image.Ready)
        compare(old.status, Image.Ready)
        const fade = findChild(art, "artCrossfade")
        verify(fade.running)
        Player.isPlaying = false
        tryCompare(fade, "running", false)
        compare(art.hasArt, true)
    }
    function test_background_freezes_veil_per_layer() {
        const bg = make(backgroundComponent, {source: teal, veil: Theme.cover.veil, veilOpacity: 0.7})
        tryCompare(bg.front, "status", Image.Ready)
        const old = bg.front
        bg.source = ""
        wait(1300)
        compare(bg.front, old)
        bg.source = green
        bg.veil = Theme.cover.base
        bg.veilOpacity = 0.9
        tryVerify(function () { return bg.front !== old })
        compare(bg.front.status, Image.Ready)
        compare(bg.under, old)
        compare(old.veilAlpha, 0.7)
        compare(bg.front.veilAlpha, 0.9)
        const incoming = bg.front
        bg.veilOpacity = 0.8
        wait(20)
        compare(incoming.veilAlpha, 0.9)
        Player.isPlaying = false
        tryCompare(findChild(bg, "backgroundCrossfade"), "running", false)
        bg.blurOn = false
        tryVerify(function () { return bg.front.status === Image.Null })
    }
    function test_flat_background_without_blur_url() {
        const bg = make(backgroundComponent, {source: "", blurOn: false})
        compare(bg.front.status, Image.Null)
        compare(bg.front.opacity, 1)
        compare(bg.driftOn, false)
        bg.veilOpacity = 0.8
        Player.isPlaying = false
        tryCompare(findChild(bg, "backgroundCrossfade"), "running", false)
    }
    function test_palette_opacity_contract() {
        const p = {base: Player.paletteBase, glow: Player.paletteGlow,
            veilDark: Player.paletteVeilDark, veilLight: Player.paletteVeilLight,
            inkDark: Player.paletteInkDark, inkLight: Player.paletteInkLight,
            veilDarkOpacity: 0.85, veilLightOpacity: 0.65}
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilOpacity, 0.85)
        Theme.mode = "light"
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilOpacity, 0.65)
        p.veilLightOpacity = 0
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilOpacity, 0)
        delete p.veilLightOpacity
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilOpacity, 1)
    }
    function test_lyrics_space_overrides_playback() {
        const lyrics = make(lyricsComponent, {model: lines, status: "synced", activeIndex: 0})
        const list = findChild(lyrics, "lyricsList")
        list.forceActiveFocus()
        keyClick(Qt.Key_Down)
        compare(list.currentIndex, 1)
        compare(lyrics.following, false)
        // A local handler forwards the request exactly like NowPlaying does.
        lyrics.seekRequested.connect(function (ms) { Player.seek(ms) })
        keyClick(Qt.Key_Space)
        compare(Player.positionMs, 2000)
        compare(Player.isPlaying, true)
        compare(lyrics.following, true)
    }
    function test_lyrics_manual_scroll_does_not_recentre_in_pause() {
        const lyrics = make(lyricsComponent, {model: lines, status: "synced", activeIndex: 0})
        lyrics.userScrolled()
        lyrics.scheduleResume()
        const resume = findChild(lyrics, "lyricsResume")
        verify(resume.running)
        Player.isPlaying = false
        compare(resume.running, false)
        const list = findChild(lyrics, "lyricsList")
        const y = list.contentY
        lyrics.activeIndex = 2
        compare(list.contentY, y)
        lyrics.trackKey = "another-song"
        compare(lyrics.following, true)
    }
    function test_volume_fine_wheel_and_no_paused_timer() {
        const volume = make(volumeComponent, {volume: 50})
        volume.wheelBy(0)
        compare(volume.shown, 50)
        for (let i = 0; i < 3; ++i) volume.wheelBy(8)
        compare(volume.shown, 51)
        Player.isPlaying = false
        compare(findChild(volume, "volumeSettle").running, false)
        volume.requestVolume(60)
        compare(findChild(volume, "volumeSettle").running, false)
    }
    function test_visualizer_reuses_buffers_and_skips_identical_frames() {
        const bands = [0.2, 0.4, 0.3, 0.6]
        const wave = make(bandComponent, {bands: bands, interpolate: false})
        tryVerify(function () { return wave.wavePoints.length > 0 })
        const buffer = wave.wavePoints
        const point = buffer[0]
        const y = point.y
        wave.bands = [0.4, 0.8, 0.6, 1.2]
        compare(wave.wavePoints, buffer)
        compare(wave.wavePoints[0], point)
        verify(point.y !== y)
        const path = findChild(wave, "vizWavePath")
        compare(path.path[0].y, point.y)
        const levels = [0.4, 0.8, 0.6, 1.2]
        verify(Viz.equal(wave.builtLevels, levels))
        wave.bands = levels
        compare(wave.wavePoints, buffer)
        compare(path.path[0].y, point.y)
    }
    function test_visualizer_stops_glide_on_pause() {
        const wave = make(bandComponent, {bands: [0.2, 0.3, 0.4]})
        wait(120)
        wave.bands = [0.6, 0.7, 0.8]
        verify(findChild(wave, "bandGlider").running)
        Player.isPlaying = false
        compare(findChild(wave, "bandGlider").running, false)
    }
    function test_seek_is_straight_without_visualizer_clock() {
        const seek = make(seekComponent)
        compare(seek.waveMoving, false)
        seek.clockRunning = true
        compare(seek.waveMoving, true)
        Player.isPlaying = false
        compare(seek.waveMoving, false)
    }
    function test_minimum_height_data() {
        return [{tag: "square", style: "normal"}, {tag: "circular", style: "circular"}]
    }
    function test_minimum_height(data) {
        Visualizer.style = data.style
        const np = make(npComponent)
        verify(np.coverSize * np.slotRatio + np.controlsHeight + np.gaps(np.compact) <= np.columnRoom)
        verify(np.coverArt.mapToItem(np, 0, 0).y >= Theme.size.titleBar)
    }
    function test_no_session_has_action_and_no_lyrics() {
        Player.hasSession = false
        const np = make(npComponent)
        compare(np.lyricsShown, false)
        compare(np.vizOn, false)
        verify(findChild(np, "npIdle").visible)
        const action = findChild(np, "npIdleAction")
        action.forceActiveFocus()
        keyClick(Qt.Key_Return)
        compare(closed, 1)
    }
    function test_performance_keep_visualizer() {
        const np = make(npComponent)
        Theme.performanceMode = true
        compare(np.vizOn, false)
        Settings.performanceKeepVisualizer = true
        compare(np.vizOn, true)
    }
}
