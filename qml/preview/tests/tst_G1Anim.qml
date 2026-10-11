import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components"
import "../../components/controls"
import "../../components/library"
import "../../theme"

// G1 (movimiento): cartel de actualización, atenuación de deshabilitados, índice de Ajustes y
// asistente de app propia. Con Modo rendimiento todo es instantáneo; con movimiento llega al
// estado final correcto.
TestCase {
    id: test
    name: "G1Anim"
    width: 1000; height: 700
    visible: true
    when: windowShown
    Component { id: noticeComponent; UpdateNotice { width: 600 } }
    Component { id: buttonComponent; LibraryButton { text: "x" } }
    Component { id: playComponent; LibraryPlayButton {} }
    Component { id: sliderComponent; VSlider {} }
    Component { id: wizardComponent; PersonalAppWizard { width: 360 } }
    Component { id: settingsComponent; SettingsPage { width: 900; height: 300 } }
    function init() {
        Theme.reduceMotion = false; Theme.performanceMode = false
        Theme.windowVisible = true; Theme.windowFocused = true; Theme.playing = true
        Backend.Updater.state = "idle"
    }
    function cleanup() { Backend.Updater.state = "idle"; Theme.performanceMode = false; Theme.reduceMotion = false }
    function make(c) { const o = createTemporaryObject(c, test); verify(o !== null); return o }

    function test_notice_instant_in_performance_mode() {
        Theme.performanceMode = true
        const n = make(noticeComponent)
        Backend.Updater.state = "available"
        compare(n.visible, true)
        compare(findChild(n, "updateCard").opacity, 1)
        Backend.Updater.state = "idle"
        compare(n.visible, false)
    }
    function test_notice_enters_and_leaves_with_motion() {
        const n = make(noticeComponent)
        Backend.Updater.state = "available"
        compare(n.visible, true)
        const card = findChild(n, "updateCard")
        tryCompare(card, "opacity", 1, 2000)
        compare(card.y, 0)
        Backend.Updater.state = "downloading"       // phase change: no second entrance
        compare(n.visible, true)
        compare(card.opacity, 1)
        Backend.Updater.state = "idle"
        tryCompare(n, "visible", false, 2000)
        // Reappears whole after an exit, whatever state the card was left in.
        Backend.Updater.state = "available"
        compare(n.visible, true)
        tryCompare(card, "opacity", 1, 2000)
        compare(card.y, 0)
    }
    function test_notice_reversible_mid_exit() {
        const n = make(noticeComponent)
        Backend.Updater.state = "available"
        tryCompare(findChild(n, "updateCard"), "opacity", 1, 2000)
        Backend.Updater.state = "idle"
        Backend.Updater.state = "available"         // back before the exit ended
        compare(n.visible, true)
        tryCompare(findChild(n, "updateCard"), "opacity", 1, 2000)
        compare(n.visible, true)
    }
    function test_disabled_dims_in_both_modes() {
        for (const perf of [true, false]) {
            Theme.performanceMode = perf
            for (const c of [buttonComponent, playComponent, sliderComponent]) {
                const o = make(c)
                compare(o.opacity, 1)
                o.enabled = false
                tryCompare(o, "opacity", Theme.library.disabledOpacity, 2000)
                o.enabled = true
                tryCompare(o, "opacity", 1, 2000)
            }
        }
    }
    function test_wizard_opens_to_full_opacity() {
        for (const perf of [true, false]) {
            Theme.performanceMode = perf
            const w = make(wizardComponent)
            const steps = findChild(w, "personalAppSteps")
            verify(steps !== null)
            w.expanded = true
            tryCompare(steps, "opacity", 1, 2000)
            w.expanded = false
            compare(steps.visible, false)
            w.expanded = true
            tryCompare(steps, "opacity", 1, 2000)
        }
    }
    function test_index_glide_lands_and_pins_selection() {
        const page = make(settingsComponent)
        const flick = findChild(page, "settingsViewport")
        verify(flick !== null)
        wait(50)
        let key = ""
        const keys = ["account", "appearance", "audio", "about", "playback", "background"]
        for (const k of keys) if (page.sectionShown(k) && page.targetY(k) > 0) { key = k }
        if (key === "") skip("sin sección alcanzable en el mock")
        page.glideTo(key)
        compare(page.navTarget, key)
        compare(page.selectedSection, key)
        tryCompare(page, "navTarget", "", 3000)
        compare(Math.round(flick.contentY), Math.round(page.targetY(key)))
    }
    function test_index_glide_instant_in_performance_mode() {
        Theme.performanceMode = true
        const page = make(settingsComponent)
        const flick = findChild(page, "settingsViewport")
        wait(50)
        let key = ""
        for (const k of ["account", "appearance", "audio", "about", "playback", "background"])
            if (page.sectionShown(k) && page.targetY(k) > 0) key = k
        if (key === "") skip("sin sección alcanzable en el mock")
        page.glideTo(key)
        compare(page.navTarget, "")
        compare(Math.round(flick.contentY), Math.round(page.targetY(key)))
    }
}
