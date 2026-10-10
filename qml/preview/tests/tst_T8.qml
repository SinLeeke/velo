import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views
import "../../components/window"
import "../../components/library"

// T8 (/impeccable of T2-T4): the sleep timer never sits under a view's buttons and uses the
// measured secondary ink; the search box keeps its minimum width and clears the timer; the cover
// is a button while there is a session; Modo rendimiento freezes the player bar's click.
TestCase {
    id: test
    name: "T8"
    when: windowShown
    visible: true
    width: 1100
    height: 760

    Component { id: barComponent; TitleBar { width: test.width; height: Theme.size.titleBar; controlsStyle: "mac" } }
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }
    Component { id: npComponent; Views.NowPlaying { width: test.width; height: 600; ownsCredits: false } }
    Component { id: playerBarComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }

    property string controlsBefore: ""
    property string barVizBefore: ""
    function initTestCase() {
        controlsBefore = Backend.Settings.windowControls
        barVizBefore = Backend.Settings.playerBarVisualizer
    }
    function init() {
        Theme.reduceMotion = true
        Theme.zoom = 1
        Theme.performanceMode = false
        Backend.Settings.performanceKeepVisualizer = false
        Backend.Settings.showLyrics = false
        Backend.Settings.playerBarVisualizer = "off"
        Backend.Account.signedIn = true
        Backend.Account.audioReady = true
        Backend.Player.hasSession = true
        Backend.Player.canControl = true
        Backend.Player.isPlaying = true
        Backend.Visualizer.enabled = true
        Backend.Visualizer.style = "normal"
        Backend.Visualizer.shape = "wave"
        Backend.Visualizer.band = true
    }
    function cleanup() {
        Backend.Actions.sleepMinutes = 0
        Backend.Actions.sleepEndsAtMs = 0
        Backend.Settings.windowControls = controlsBefore
    }
    function cleanupTestCase() {
        Backend.Settings.playerBarVisualizer = barVizBefore
        Backend.Settings.showLyrics = true
        Theme.reduceMotion = false
        Theme.performanceMode = false
        Theme.zoom = 1
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }
    function vizState() {
        return [Backend.Visualizer.enabled, Backend.Visualizer.style, Backend.Visualizer.shape].join("/")
    }

    // ---- sleep timer in the title bar
    function test_timer_stays_left_of_the_view_buttons() {
        const bar = make(barComponent)
        const timer = findChild(bar, "titleSleepTimer")
        verify(timer !== null)
        Backend.Actions.sleepMinutes = 30
        Backend.Actions.sleepEndsAtMs = Date.now() + 12 * 60000
        verify(timer.visible)
        compare(bar.timerWidth, timer.width)
        bar.trailingReserve = 140
        verify(timer.x + timer.width <= bar.width - 140, "clear of the reserved strip")
        // With Windows caption buttons the larger of both wins.
        bar.controlsStyle = "windows"
        bar.trailingReserve = 0
        verify(timer.x + timer.width <= bar.width - bar.trailingInset, "clear of the caption buttons")
        Backend.Actions.sleepMinutes = 0
        compare(bar.timerWidth, 0)
    }
    function test_timer_uses_secondary_ink_not_opacity() {
        const bar = make(barComponent)
        Backend.Actions.sleepMinutes = -1
        const timer = findChild(bar, "titleSleepTimer")
        compare(timer.opacity, 1)
        bar.inkSecondary = Theme.coverInkSecondary
        let label = null
        for (let i = 0; i < timer.children.length; ++i)
            if (timer.children[i].text !== undefined) label = timer.children[i]
        verify(label !== null)
        verify(Qt.colorEqual(label.color, Theme.coverInkSecondary))
        verify(timer.Accessible.name.indexOf(qsTr("Temporizador")) === 0, "named as the timer")
    }
    function test_library_and_now_playing_reserve_their_buttons() {
        const library = make(libraryComponent)
        const settings = findChild(library, "openSettingsButton")
        verify(library.titleTrailingReserve >= library.width - settings.x, "library: Ajustes and tema")
        const np = make(npComponent)
        verify(np.titleTrailingReserve > Theme.size.toolPill, "Now Playing: the tool pill")
    }

    // ---- search box in the top bar
    function test_search_box_clears_the_timer() {
        const library = make(libraryComponent)
        const box = library.searchBox
        const settings = findChild(library, "openSettingsButton")
        library.titleTimerWidth = 120
        wait(0)
        // The timer's left edge: settings.x - md - width; the box ends a gap before it.
        verify(box.x + box.width <= settings.x - Theme.space.md - 120 - Theme.searchBox.gap + 1)
        library.titleTimerWidth = 0
    }
    function test_search_box_keeps_its_minimum_width() {
        Backend.Settings.windowControls = "windows"
        const library = make(libraryComponent)
        library.width = Theme.size.windowMinWidth
        library.sidebarDragWidth = Theme.size.sidebarMax
        wait(0)
        verify(library.searchBox.width >= Theme.searchBox.minWidth,
               "box " + library.searchBox.width + " >= " + Theme.searchBox.minWidth)
        verify(library.sidebarWidth >= Theme.size.sidebarMin)
        library.sidebarDragWidth = -1
    }

    function test_search_box_hit_target_and_strip_at_zoom() {
        for (const z of [0.75, 1.5]) {
            Theme.zoom = z
            verify(Theme.searchBox.height >= Theme.size.minimumHitTarget, "zoom " + z)
            verify(Theme.searchBox.height <= Theme.size.titleBar, "inside the strip at zoom " + z)
        }
        Theme.zoom = 1
    }

    // ---- cover button
    function test_cover_enabled_with_session_without_a_visualizer() {
        const np = make(npComponent)
        const button = findChild(np, "npCoverButton")
        verify(button.enabled)
        Backend.Visualizer.enabled = false
        wait(0)
        verify(button.enabled, "the cover shape can change without a visualizer")
        mouseClick(np.coverArt)
        compare(vizState(), "false/circular/wave")
        Backend.Visualizer.enabled = true
        Theme.performanceMode = true
        wait(0)
        verify(button.enabled, "the cover shape can change in Modo rendimiento")
        mouseClick(np.coverArt)
        compare(vizState(), "true/normal/wave")
        Backend.Settings.performanceKeepVisualizer = true
        wait(0)
        verify(button.enabled)
        mouseClick(np.coverArt)
        compare(vizState(), "true/circular/wave")
        Backend.Player.hasSession = false
        wait(0)
        verify(!button.enabled, "no session: the cover is not a button")
        mouseClick(np.coverArt)
        compare(vizState(), "true/circular/wave")
    }

    // ---- player bar under Modo rendimiento
    function test_player_bar_click_frozen_in_performance_mode() {
        const bar = make(playerBarComponent)
        let p = null
        for (let y = 2; y < bar.height && !p; y += 4)
            for (let x = 2; x < bar.width && !p; x += 8)
                if (bar.freeAt(x, y)) p = Qt.point(x, y)
        verify(p !== null)
        Theme.performanceMode = true
        mouseClick(bar, p.x, p.y)
        compare(Backend.Settings.playerBarVisualizer, "off")
        compare(bar.Accessible.description, "")
        Backend.Settings.performanceKeepVisualizer = true
        mouseClick(bar, p.x, p.y)
        compare(Backend.Settings.playerBarVisualizer, "bars")
    }
}
