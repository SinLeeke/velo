import QtQuick
import QtTest
import "../../theme"
import "../../components/library"

// G2: SectionReveal (sections of a page) and FirstDataFade (one fade of a grid on first data).
Item {
    width: 300; height: 300

    Column {
        id: sec
        width: 100; height: 40
        visible: false
        transform: Translate { id: secShift }
        SectionReveal { id: reveal; item: sec; shift: secShift; order: 1 }
    }
    Column {
        id: secAtOpen
        width: 100; height: 40
        transform: Translate { id: atOpenShift }
        SectionReveal { id: atOpen; item: secAtOpen; shift: atOpenShift; order: 0 }
    }
    Item { id: grid; width: 100; height: 100 }
    FirstDataFade { id: fade; target: grid; count: 0 }
    Item { id: grid2; width: 100; height: 100 }
    FirstDataFade { id: fade2; target: grid2; count: 5 }

    TestCase {
        name: "G2Motion"
        when: windowShown

        function cleanup() { Theme.performanceMode = false; Theme.reduceMotion = false; Theme.playing = true }

        function test_section_enters_when_it_becomes_visible_late() {
            verify(Theme.motion.base > 0)
            sec.visible = true
            compare(sec.opacity, 0)
            compare(secShift.y, Theme.entrance.cardShift)
            tryCompare(sec, "opacity", 1, 2000)
            tryCompare(secShift, "y", 0, 2000)
            verify(reveal.done)
        }

        function test_section_present_at_open_does_not_animate() {
            compare(secAtOpen.opacity, 1)
            compare(atOpenShift.y, 0)
            verify(!atOpen.running)
        }

        function test_section_is_instant_in_performance_mode() {
            Theme.performanceMode = true
            compare(Theme.motion.base, 0)
            sec.visible = false
            reveal.done = false
            sec.opacity = 1
            sec.visible = true
            compare(sec.opacity, 1)
            compare(secShift.y, 0)
        }

        function test_first_data_fades_grid_once() {
            compare(fade.armed, true)
            fade.count = 12
            compare(grid.opacity, 0)
            tryCompare(grid, "opacity", 1, 2000)
            fade.count = 24
            compare(grid.opacity, 1)
            verify(!fade.running)
        }

        function test_data_already_there_does_not_fade() {
            compare(fade2.armed, false)
            fade2.count = 9
            compare(grid2.opacity, 1)
        }

        function test_settle_leaves_final_state() {
            grid.opacity = 0.3
            fade.settle()
            compare(grid.opacity, 1)
        }

        function test_first_data_is_instant_without_motion() {
            Theme.reduceMotion = true
            const g = createTemporaryObject(probe, this)
            g.fade.count = 3
            compare(g.opacity, 1)
        }
    }

    Component {
        id: probe
        Item {
            property alias fade: f
            FirstDataFade { id: f; target: parent; count: 0 }
        }
    }
}
