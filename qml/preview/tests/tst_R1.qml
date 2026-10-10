import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../nav"
import "../../views" as Views
import "../../components"
import "../../components/controls"
import "../../components/library"
import "../../components/window"

// R1: behavioral checks only, with the backend mocks. No captures or network.
TestCase {
    id: test
    name: "R1"
    when: windowShown
    visible: true
    width: 1100
    height: 760
    property int globalActivations: 0
    Item { id: elsewhere; width: 20; height: 20; focus: true }
    // Same focus gate used by Main: the focused control must receive the real key event.
    Shortcut {
        sequence: "Space"
        enabled: test.Window.window !== null && !Nav.focusUsesKeys(test.Window.window.activeFocusItem)
        onActivated: { ++test.globalActivations; Backend.Player.playPause() }
    }
    VButton { id: button; x: 40; y: 60; kind: "accent"; text: qsTr("Reproducir") }
    LibraryButton { id: libraryButton; x: 200; y: 60; text: qsTr("Abrir") }
    LibraryChip { id: chip; x: 340; y: 60; text: qsTr("Álbumes") }
    IconButton { id: icon; x: 480; y: 60; text: qsTr("Aleatorio"); toggle: true; iconSource: Qt.resolvedUrl("../../icons/shuffle.svg") }
    VToggle { id: toggle; x: 560; y: 60; text: qsTr("Efectos") }
    VTextField { id: field; x: 40; y: 140; width: 240; accessibleName: qsTr("Nombre") }
    VSegmented {
        id: segmented
        x: 340; y: 140; name: qsTr("Estilo")
        model: [{value: "a", label: qsTr("Barras")}, {value: "b", label: qsTr("Onda")}, {value: "c", label: qsTr("Circular")}]
        current: "a"
        onActivated: value => current = value
    }
    ResizeHandle {
        id: resize
        x: 700; y: 60; height: 140; minimum: 200; maximum: 400; value: 240
        onResized: width => value = width
        onResetRequested: value = 240
    }
    Toast { id: toast }
    SignalSpy { id: clicks; signalName: "clicked" }
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }
    Component { id: loginComponent; Views.Login { width: test.width; height: test.height } }
    Component { id: wizardComponent; PersonalAppWizard { width: 360 } }
    Component { id: trafficComponent; TrafficLights { x: 50; y: 240 } }
    Component {
        id: chromeComponent
        Item {
            width: 500; height: 100; x: 100; y: 240
            property alias buttons: caption
            WindowButtons { id: caption; anchors.right: parent.right; y: 10; height: Theme.size.titleBar }
        }
    }

    function init() {
        Theme.mode = "dark"; Theme.zoom = 1
        Theme.performanceMode = true; Theme.reduceMotion = false
        Theme.windowVisible = true; Theme.windowFocused = true; Theme.playing = true
        Theme.pauseInBackground = false
        Backend.Player.isPlaying = true; Backend.Player.canControl = true
        Backend.Account.signedIn = true; Backend.Account.audioReady = true; Backend.Account.busy = false
        globalActivations = 0
        icon.active = false; toggle.checked = false; field.text = ""
        segmented.current = "a"; segmented.LayoutMirroring.enabled = false
        resize.growsRight = true; resize.value = 240
        toast.dismiss(); clicks.clear()
        mouseMove(elsewhere); elsewhere.forceActiveFocus()
    }
    function cleanup() {
        toast.dismiss(); clicks.target = null
        Theme.zoom = 1; Theme.playing = true; Theme.windowVisible = true; Theme.windowFocused = true
        Backend.Player.isPlaying = true; Backend.Account.signedIn = true; Backend.Account.busy = false
    }
    function test_space_activates_controls_and_only_plays_from_background() {
        for (const control of [button, libraryButton, chip, icon]) {
            clicks.target = control; clicks.clear()
            control.forceActiveFocus(Qt.TabFocusReason)
            verify(Nav.focusUsesKeys(test.Window.window.activeFocusItem))
            keyClick(Qt.Key_Space)
            compare(clicks.count, 1)
            compare(globalActivations, 0)
            compare(Backend.Player.isPlaying, true)
        }
        toggle.forceActiveFocus(Qt.TabFocusReason); keyClick(Qt.Key_Space)
        compare(toggle.checked, true); compare(globalActivations, 0)
        field.forceActiveFocus(Qt.TabFocusReason); keyClick(Qt.Key_Space)
        compare(field.text, " "); compare(globalActivations, 0)
        elsewhere.forceActiveFocus(); keyClick(Qt.Key_Space)
        compare(globalActivations, 1); compare(Backend.Player.isPlaying, false)
    }
    function test_enter_and_disabled_controls() {
        for (const control of [button, libraryButton, icon]) {
            clicks.target = control; clicks.clear()
            control.forceActiveFocus(Qt.TabFocusReason); keyClick(Qt.Key_Return)
            compare(clicks.count, 1)
            control.enabled = false; clicks.clear()
            keyClick(Qt.Key_Return); compare(clicks.count, 0)
            control.enabled = true
        }
    }
    function test_toggle_accessibility_and_primary_focus() {
        verify(icon.Accessible.checkable); compare(icon.Accessible.checked, false)
        icon.active = true; compare(icon.Accessible.checked, true)
        compare(field.Accessible.name, qsTr("Nombre"))
        button.forceActiveFocus(Qt.TabFocusReason); verify(button.visualFocus)
        compare(button.primary, true)
        verify(contrast(Theme.library.focusOnAccent, Theme.accent) >= 3)
        button.enabled = false; compare(button.opacity, Theme.library.disabledOpacity); button.enabled = true
        compare(button.implicitHeight, libraryButton.implicitHeight)
        compare(field.implicitHeight, Theme.size.titleBar)
    }
    function test_segmented_options_keyboard_and_rtl() {
        segmented.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Right); compare(segmented.current, "b")
        keyClick(Qt.Key_End); compare(segmented.current, "c")
        keyClick(Qt.Key_Home); compare(segmented.current, "a")
        segmented.LayoutMirroring.enabled = true
        keyClick(Qt.Key_Left); compare(segmented.current, "b")
        const radios = []
        function visit(item) {
            if (item.Accessible.role === Accessible.RadioButton) radios.push(item)
            for (const child of item.children) visit(child)
        }
        visit(segmented)
        compare(radios.length, 3)
        compare(radios[1].Accessible.checked, true)
        radios[2].Accessible.pressAction(); compare(segmented.current, "c")
    }
    function test_resize_keyboard_and_accessible_range() {
        resize.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Right); compare(resize.value, 240 + Theme.resize.keyStep)
        keyClick(Qt.Key_Home); compare(resize.value, 200)
        keyClick(Qt.Key_End); compare(resize.value, 400)
        keyClick(Qt.Key_Delete); compare(resize.value, 240)
        resize.growsRight = false
        keyClick(Qt.Key_Right); compare(resize.value, 240 - Theme.resize.keyStep)
        compare(resize.minimumValue, 200); compare(resize.maximumValue, 400)
        compare(resize.Accessible.role, Accessible.Slider)
    }
    function test_snap_rect_geometry_zoom_hidden_and_destroyed() {
        const frame = createTemporaryObject(chromeComponent, test)
        verify(frame !== null)
        const b = frame.buttons
        tryVerify(() => b.maximizeButton !== null)
        function checkRect() {
            const zoom = b.maximizeButton
            const p = zoom.mapToItem(null, 0, 0)
            tryCompare(Backend.Desktop, "maximizeButtonRect", Qt.rect(p.x, p.y, zoom.width, zoom.height))
        }
        checkRect(); frame.x += 30; frame.y += 20; checkRect()
        frame.width += 100; checkRect()
        Theme.zoom = 0.75; wait(0); checkRect()
        b.visible = false
        compare(Backend.Desktop.maximizeButtonRect, Qt.rect(0, 0, 0, 0))
        b.visible = true; checkRect()
        frame.destroy(); wait(0)
        compare(Backend.Desktop.maximizeButtonRect, Qt.rect(0, 0, 0, 0))
    }
    function test_tooltip_and_small_zoom_target() {
        Theme.zoom = 0.75
        verify(icon.width >= Theme.size.minimumHitTarget)
        mouseMove(icon)
        tryVerify(() => findChild(icon, "iconToolTip") !== null)
        const tip = findChild(icon, "iconToolTip")
        compare(tip.text, icon.text); compare(tip.delay, Theme.tip.delay)
        tryCompare(tip, "opened", true)
        Theme.windowVisible = false
        tryVerify(() => findChild(icon, "iconToolTip") === null)
    }
    function test_traffic_lights_keep_disjoint_minimum_hit_cells() {
        const lights = createTemporaryObject(trafficComponent, test)
        const cells = []
        function visit(item) {
            if (item instanceof MouseArea) cells.push(item)
            for (const child of item.children) visit(child)
        }
        visit(lights); compare(cells.length, 3)
        for (const zoom of [0.75, 1, 1.5]) {
            Theme.zoom = zoom
            let right = -Infinity
            for (const cell of cells) {
                verify(cell.width >= Theme.size.minimumHitTarget)
                verify(cell.height >= Theme.size.minimumHitTarget)
                const x = cell.mapToItem(null, 0, 0).x
                verify(x >= right); right = x + cell.width
            }
        }
    }
    function test_personal_app_wizard_is_collapsed_until_requested() {
        const wizard = createTemporaryObject(wizardComponent, test)
        verify(wizard !== null); compare(wizard.expanded, false)
        const clientId = findChild(wizard, "personalClientId")
        verify(!clientId.visible)
        const configure = findChild(wizard, "configurePersonalApp")
        configure.forceActiveFocus(Qt.TabFocusReason); keyClick(Qt.Key_Return)
        compare(wizard.expanded, true); verify(clientId.visible)
        keyClick(Qt.Key_Return); compare(wizard.expanded, false)
    }
    function test_toast_focus_holds_deadline_and_pause_keeps_message() {
        toast.show(qsTr("No se pudo guardar. Vuelve a intentarlo."), "", "error")
        compare(toast.Accessible.role, Accessible.AlertMessage)
        mouseMove(toast)
        verify(toast.held)
        wait(Theme.toast.timeout + 50)
        verify(toast.shown)
        mouseMove(elsewhere)
        Backend.Player.isPlaying = false
        wait(Theme.toast.timeout + 50)
        verify(toast.shown)
        toast.show(qsTr("Aviso"), qsTr("Volver a intentar"), "notice")
        compare(toast.isError, false)
        const action = findChild(toast, "toastAction")
        action.forceActiveFocus(Qt.TabFocusReason); verify(toast.held)
        keyClick(Qt.Key_Return); compare(toast.shown, false)
    }
    function test_login_initial_focus_error_and_retry() {
        Backend.Account.signedIn = false
        const login = createTemporaryObject(loginComponent, test)
        verify(login !== null); login.reveal(false)
        const signIn = findChild(login, "signInButton")
        tryCompare(signIn, "activeFocus", true)
        Backend.Account.signInFailed(qsTr("No se pudo conectar. Comprueba la conexión y vuelve a intentarlo."))
        const banner = findChild(login, "loginToast")
        verify(banner.shown); compare(banner.actionText, qsTr("Volver a intentar"))
        findChild(banner, "toastAction").forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Return)
        compare(Backend.Account.signedIn, true)
    }
    function test_history_keeps_page_instances_scroll_and_mouse_buttons() {
        const library = createTemporaryObject(libraryComponent, test)
        verify(library !== null)
        const home = library.currentPageItem
        library.navigate("settings", "", "")
        const settings = library.currentPageItem
        const flick = library.mainFlickable(settings)
        verify(flick !== null); verify(flick.contentHeight > flick.height)
        flick.contentY = flick.originY + 180
        library.navigate("liked", "", "")
        const tracks = library.currentPageItem
        library.goBack()
        compare(library.currentPageItem, settings)
        tryCompare(flick, "contentY", flick.originY + 180)
        library.goForward(); compare(library.currentPageItem, tracks)
        // Going through a local page preserves the same authoritative list in Rust.
        const title = Backend.Library.currentTitle
        library.navigate("settings", "account", ""); library.goBack()
        compare(library.currentPageItem, tracks); compare(Backend.Library.currentTitle, title)
        mouseClick(library, library.width / 2, library.height / 2, Qt.BackButton)
        compare(library.currentPageItem, settings)
        mouseClick(library, library.width / 2, library.height / 2, Qt.ForwardButton)
        compare(library.currentPageItem, tracks)
        library.goBack(); library.goBack()
        compare(library.currentPageItem, home)
        compare(Object.keys(library.pageCache).length, 3)
    }
    function luminance(c) {
        function linear(v) { return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
    }
    function contrast(a, b) {
        const x = luminance(a), y = luminance(b)
        return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)
    }
    function test_theme_contrast_and_palette_veil_opacity() {
        for (const mode of ["dark", "light"]) {
            Theme.mode = mode
            for (const surface of [Theme.bg, Theme.surface1, Theme.surface2, Theme.surface3]) {
                verify(contrast(Theme.textTertiary, surface) >= 4.5, mode + " tertiary")
                verify(contrast(Theme.accent, surface) >= 4.5, mode + " accent")
                verify(contrast(Theme.controlBorder, surface) >= 3, mode + " control border")
            }
            Theme.setCoverPalette({base: Backend.Player.paletteBase, glow: Backend.Player.paletteGlow,
                veilDark: Backend.Player.paletteVeilDark, veilLight: Backend.Player.paletteVeilLight,
                inkDark: Backend.Player.paletteInkDark, inkLight: Backend.Player.paletteInkLight,
                veilDarkOpacity: 0.91, veilLightOpacity: 0.95})
            compare(Theme.cover.veilOpacity, mode === "dark" ? 0.91 : 0.95)
        }
    }
    function test_effects_stop_on_pause_minimize_and_background_preference() {
        Theme.performanceMode = false; verify(Theme.motion.fast > 0)
        Theme.playing = false; compare(Theme.motion.fast, 0); compare(Theme.motion.coverFade, 0)
        Theme.playing = true; Theme.windowVisible = false; compare(Theme.motion.fast, 0)
        Theme.windowVisible = true; Theme.windowFocused = false; verify(Theme.motion.fast > 0)
        Theme.pauseInBackground = true; compare(Theme.motion.fast, 0)
    }
}
