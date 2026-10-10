import QtQuick
import QtTest
import Velo.Backend
import "../../views" as Views
import "../../components/library"
import "../../theme"

// T4: clicks that switch the visualizer. State only: no screenshots, network or Rust.
TestCase {
    id: test
    name: "T4"
    visible: true
    width: 1100
    height: 680
    when: windowShown

    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }
    Component {
        id: npComponent
        Views.NowPlaying { width: test.width; height: 600; ownsCredits: false }
    }

    property string savedBarViz
    function initTestCase() {
        savedBarViz = Settings.playerBarVisualizer
    }
    function init() {
        Theme.reduceMotion = true
        Theme.zoom = 1
        Theme.performanceMode = false
        Settings.performanceKeepVisualizer = false
        Settings.showLyrics = false
        Settings.playerBarVisualizer = "off"
        Player.hasSession = true
        Player.canControl = true
        Player.isPlaying = true
        Player.durationMs = 200000
        Player.positionMs = 60000
        Visualizer.enabled = true
        Visualizer.style = "normal"
        Visualizer.shape = "wave"
    }
    function cleanupTestCase() {
        Settings.playerBarVisualizer = savedBarViz
        Settings.showLyrics = true
        Visualizer.enabled = true
        Visualizer.style = "normal"
        Visualizer.shape = "wave"
        Theme.reduceMotion = false
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }
    // First point of a coarse grid where `ok(x, y)` holds (item coordinates).
    function findPoint(item, ok) {
        for (let y = 2; y < item.height; y += 4)
            for (let x = 2; x < item.width; x += 8)
                if (ok(x, y)) return Qt.point(x, y)
        return null
    }
    function vizState() { return [Visualizer.enabled, Visualizer.style, Visualizer.shape].join("/") }

    // ---- library player bar
    function test_bar_free_click_cycles() {
        const bar = make(barComponent)
        const p = findPoint(bar, (x, y) => bar.freeAt(x, y))
        verify(p !== null, "the bar has free space")
        mouseClick(bar, p.x, p.y)
        compare(Settings.playerBarVisualizer, "bars")
        mouseClick(bar, p.x, p.y)
        compare(Settings.playerBarVisualizer, "wave")
        mouseClick(bar, p.x, p.y)
        compare(Settings.playerBarVisualizer, "off")
    }
    function test_bar_button_and_near_miss_do_not_cycle() {
        const bar = make(barComponent)
        Player.canControl = false          // disabled buttons let the press through
        const play = findChild(bar, "playerBarPlay")
        const c = play.mapToItem(bar, play.width / 2, play.height / 2)
        mouseClick(bar, c.x, c.y)
        compare(Settings.playerBarVisualizer, "off")
        // Just beside the play button (inside the margin), not on any control.
        const m = Theme.nowPlayingClicks.barMargin
        const corner = play.mapToItem(bar, 0, 0)
        const nearX = corner.x - Math.max(1, m / 2)
        verify(!bar.freeAt(nearX, c.y))
        mouseClick(bar, nearX, c.y)
        compare(Settings.playerBarVisualizer, "off")
        Player.canControl = true
        mouseClick(play)
        compare(Settings.playerBarVisualizer, "off")
        Player.isPlaying = true
    }
    function test_bar_accessible_mentions_visualizer() {
        const bar = make(barComponent)
        verify(bar.Accessible.description.indexOf(qsTr("visualizador")) >= 0)
    }

    // ---- Now Playing
    function backgroundPoint(np) {
        const cover = np.coverArt
        return findPoint(np, (x, y) => {
            if (y < Theme.size.titleBar + Theme.space.lg || !np.freeAt(x, y)) return false
            const q = cover.mapFromItem(np, x, y)
            return q.x < -Theme.space.lg || q.y < -Theme.space.lg
                || q.x > cover.width + Theme.space.lg || q.y > cover.height + Theme.space.lg
        })
    }
    function test_background_click_rotates() {
        const np = make(npComponent)
        const p = backgroundPoint(np)
        verify(p !== null, "Now Playing has free background")
        mouseClick(np, p.x, p.y)
        compare(vizState(), "true/circular/wave")
        compare(np.circular, true)
        mouseClick(np, p.x, p.y)
        compare(Visualizer.enabled, false)
        compare(np.vizOn, false)
        mouseClick(np, p.x, p.y)
        compare(vizState(), "true/normal/wave")
        compare(np.bandShown, true)
    }
    function test_cover_click_toggles_shape() {
        const np = make(npComponent)
        mouseClick(np.coverArt)
        compare(vizState(), "true/normal/bars")
        mouseClick(np.coverArt)
        compare(vizState(), "true/normal/wave")
        // Round cover: the centre is the art, the slot's corner (ring area) is background.
        Visualizer.style = "circular"
        wait(0)
        mouseClick(np.coverArt)
        compare(vizState(), "true/circular/bars")
        mouseClick(np.coverArt, 2, 2)
        compare(vizState(), "false/circular/bars")
    }
    function test_cover_keyboard_and_name() {
        const np = make(npComponent)
        const button = findChild(np, "npCoverButton")
        verify(button.Accessible.name.length > 0)
        button.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Return)
        compare(Visualizer.shape, "bars")
        keyClick(Qt.Key_Space)
        compare(Visualizer.shape, "wave")
    }
    function test_buttons_do_not_rotate() {
        const np = make(npComponent)
        mouseClick(findChild(np, "npCloseButton"))
        compare(vizState(), "true/normal/wave")
        // Disabled transport, progress and volume let the press through: still not background.
        Player.canControl = false
        for (const name of ["npSeekBar", "npTransport", "npVolume"]) {
            const it = findChild(np, name)
            verify(it !== null, name)
            mouseClick(it, it.width / 2, it.height / 2)
            compare(vizState(), "true/normal/wave", name)
        }
    }
    function test_click_that_closes_menu_does_not_rotate() {
        const np = make(npComponent)
        const p = backgroundPoint(np)
        np.songMenu.popup(np, np.width / 2, np.height / 2)
        tryCompare(np.songMenu, "opened", true)
        mouseClick(np, p.x, p.y)
        tryCompare(np.songMenu, "visible", false)
        compare(vizState(), "true/normal/wave")
        wait(0)                            // the next click comes after an event loop turn
        mouseClick(np, p.x, p.y)
        compare(vizState(), "true/circular/wave")
    }
    // T4b: the press that closes a popup is neither a cover click nor a background click.
    function test_click_on_cover_that_closes_popup_keeps_shape() {
        const np = make(npComponent)
        np.songMenu.popup(np, np.width / 2, np.height / 2)
        tryCompare(np.songMenu, "opened", true)
        mouseClick(np.coverArt)
        tryCompare(np.songMenu, "visible", false)
        compare(vizState(), "true/normal/wave")
        wait(0)
        // The device picker (a non-modal T.Popup) too.
        mouseClick(findChild(np, "npDevicesButton"))
        wait(0)
        mouseClick(np.coverArt)
        compare(vizState(), "true/normal/wave")
        wait(0)
        mouseClick(np.coverArt)
        compare(vizState(), "true/normal/bars")
    }
    function test_lifted_cover_rim_is_the_cover() {
        const np = make(npComponent)
        const cover = np.coverArt
        cover.tiltEnabled = false
        cover.lift = Theme.motion.tiltLift
        const ext = cover.artSize * (Theme.motion.tiltLift - 1) / 2
        verify(ext >= 2)
        mouseClick(cover, (cover.width - cover.artSize) / 2 - ext / 2, cover.height / 2)
        compare(vizState(), "true/normal/bars")
    }
    function test_bar_click_that_closes_popup_does_not_cycle() {
        const bar = make(barComponent)
        const p = findPoint(bar, (x, y) => bar.freeAt(x, y))
        bar.vizMenu.popup(bar, bar.width / 2, 0)
        tryCompare(bar.vizMenu, "opened", true)
        mouseClick(bar, p.x, p.y)
        tryCompare(bar.vizMenu, "visible", false)
        compare(Settings.playerBarVisualizer, "off")
        wait(0)
        const output = findChild(bar, "playerBarOutput").outputPopup
        output.open()
        tryCompare(output, "opened", true)
        mouseClick(bar, p.x, p.y)
        tryCompare(output, "visible", false)
        compare(Settings.playerBarVisualizer, "off")
        wait(0)
        mouseClick(bar, p.x, p.y)
        compare(Settings.playerBarVisualizer, "bars")
    }
    function test_bar_margin_has_a_floor_at_low_zoom() {
        Theme.zoom = 0.75
        verify(Theme.nowPlayingClicks.barMargin >= 8)
        Theme.zoom = 1
    }
    function test_forced_off_click_does_nothing() {
        const np = make(npComponent)
        Theme.performanceMode = true
        compare(np.vizOn, false)
        const p = backgroundPoint(np)
        mouseClick(np, p.x, p.y)
        compare(vizState(), "true/normal/wave")
        Settings.performanceKeepVisualizer = true
        mouseClick(np, p.x, p.y)
        compare(vizState(), "true/circular/wave")
    }
    function test_volume_track_lines_up_with_progress_track() {
        const np = make(npComponent)
        const seekBar = findChild(np, "npSeekBar")
        const volume = findChild(np, "npVolume")
        compare(volume.width, seekBar.width)
        compare(volume.mapToItem(np, 0, 0).x, seekBar.mapToItem(np, 0, 0).x)
        compare(volume.trackInset, seekBar.trackInset)
        verify(seekBar.trackInset > 0)
    }
}
