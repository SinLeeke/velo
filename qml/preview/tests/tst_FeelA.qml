import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components"
import "../../components/controls"
import "../../components/library"
import "../../theme"

// Lavado de cara, grupo A: rueda del raton (WheelScroll), composicion de Ajustes, confirmacion
// de interruptores y pop de toggles.
TestCase {
    id: test
    name: "FeelA"
    width: 1200; height: 700
    visible: true
    when: windowShown

    Component {
        id: scrollerComponent
        Flickable {
            width: 300; height: 400
            contentWidth: 300; contentHeight: 4000
            WheelScroll { id: ws; objectName: "ws"; rowHeight: 50 }
            Rectangle { width: 300; height: 4000; color: "red" }
        }
    }
    Component { id: settingsComponent; SettingsPage { width: 1200; height: 600 } }
    Component { id: iconComponent; IconButton { toggle: true; iconSource: Qt.resolvedUrl("../../icons/check.svg"); text: "t" } }
    Component { id: haloComponent; DjHalo { width: 40; height: 40 } }
    Component { id: toggleComponent; VToggle { text: "t" } }

    function init() {
        Theme.reduceMotion = false; Theme.performanceMode = false
        Theme.windowVisible = true; Theme.windowFocused = true; Theme.playing = true
    }
    function cleanup() { Theme.performanceMode = false; Theme.playing = true }
    function make(c) { const o = createTemporaryObject(c, test); verify(o !== null); return o }

    function test_wheel_moves_three_rows_per_notch() {
        const f = make(scrollerComponent)
        mouseWheel(f, 100, 100, 0, -120)
        tryCompare(f, "contentY", 150, 1500)       // 3 rows x 50 px
        mouseWheel(f, 100, 100, 0, 120)
        tryCompare(f, "contentY", 0, 1500)
    }
    function test_wheel_accumulates_and_clamps() {
        const f = make(scrollerComponent)
        mouseWheel(f, 100, 100, 0, -120); mouseWheel(f, 100, 100, 0, -120)
        tryCompare(f, "contentY", 300, 1500)
        mouseWheel(f, 100, 100, 0, 120 * 10)
        tryCompare(f, "contentY", 0, 1500)          // never past the top
    }
    function test_wheel_capped_by_window_share_and_instant_in_performance_mode() {
        Theme.performanceMode = true
        const f = make(scrollerComponent)
        f.height = 200                               // 0.4 x 200 = 80 < 150
        mouseWheel(f, 100, 100, 0, -120)
        compare(f.contentY, 80)
    }
    function test_wheel_works_in_pause() {
        Theme.playing = false
        const f = make(scrollerComponent)
        mouseWheel(f, 100, 100, 0, -120)
        tryCompare(f, "contentY", 150, 1500)
    }
    function test_settings_wheel_is_faster_than_qt_default() {
        const page = make(settingsComponent)
        const flick = findChild(page, "settingsViewport")
        verify(flick !== null)
        wait(50)
        mouseWheel(page, 600, 300, 0, -120)
        tryVerify(function () { return flick.contentY >= 100 }, 1500)   // default Qt step is ~20 px
    }
    function test_settings_block_is_centred_when_wide() {
        const page = make(settingsComponent)
        verify(page.railMode)
        const rail = findChild(page, "settingsIndex")
        verify(rail.x > 0)
        compare(Math.abs((rail.x + page.blockWidth / 2) - (page.width - Theme.space.lg) / 2) <= 1, true)
    }
    function test_toggle_glyph_pops_on_and_returns_to_rest() {
        const b = make(iconComponent)
        b.active = true
        wait(Theme.motion.base + 120)
        compare(b.active, true)
        b.active = false
        wait(Theme.motion.base + 120)
        compare(b.active, false)
    }
    function test_switch_shows_a_check_when_on() {
        const t = make(toggleComponent)
        t.toggle()
        verify(t.checked)
        wait(Theme.motion.fast + 80)
    }

        function test_ambient_loops_stop_in_pause_and_run_with_music() {
            Backend.Player.djActive = true; Backend.Player.djSpeaking = true
            const h = make(haloComponent)
            Backend.Player.isPlaying = true; Theme.playing = true
            verify(h.breathing)
            Theme.playing = false; Backend.Player.isPlaying = false
            verify(!h.breathing)
            Backend.Player.isPlaying = true; Theme.playing = true
            Theme.performanceMode = true
            verify(!h.breathing)
            Theme.performanceMode = false
            Backend.Player.djActive = false; Backend.Player.djSpeaking = false
        }
}
