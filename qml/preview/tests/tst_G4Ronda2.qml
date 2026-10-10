import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components"
import "../../components/library"
import "../../components/nowplaying"
import "../../components/window"
import "../../theme"

// G4 (ronda 2): the volume panel always folds, the bar visualizer stays inside its box and glides
// between slow updates, the side-panel resize handle, and the window buttons going through the
// native commands (G5). No captures.
// qmltestrunner -import qml/mock -import qml -input qml/preview/tests/tst_G4Ronda2.qml
TestCase {
    id: test
    name: "G4Ronda2"
    visible: true
    width: 960
    height: 640
    when: windowShown

    Rectangle { anchors.fill: parent; color: Theme.bg }
    Item { id: elsewhere; width: 20; height: 20; focus: true }
    MouseArea { id: target; x: 40; y: 300; width: 50; height: 50; property int hits: 0; onPressed: hits++ }
    VolumeButton { id: volume; x: test.width - width * 3; y: test.height - height }

    WindowButtons { id: caption; x: test.width - width; y: 0 }

    function loud(v) {
        const a = []
        for (let i = 0; i < 48; ++i) a.push(v * (0.6 + 0.6 * Math.sin(i)))
        return a
    }
    VisualizerBand { id: wave; x: 20; y: 20; width: 600; height: 80; shape: "wave"; outline: true; bands: test.loud(1.4) }

    Rectangle { id: panel; x: 100; y: 120; width: 232; height: 150; color: Theme.surface1 }
    ResizeHandle {
        id: sizer
        x: panel.x + panel.width - width / 2
        y: panel.y
        height: panel.height
        minimum: 200
        maximum: 420
        value: panel.width
        onResized: w => panel.width = w
        onCommitted: w => panel.width = w
        onResetRequested: panel.width = 232
    }

    function init() {
        Backend.Player.canControl = true
        Backend.Player.volumeLatencyMs = 0
        mouseMove(elsewhere)
        elsewhere.forceActiveFocus()
        volume.dismissed = false
        tryCompare(volume, "open", false)
    }

    function test_volume_hover_slop() {
        mouseMove(volume)
        tryCompare(volume, "open", true)
        mouseMove(volume, -Theme.audioOutput.volumeHoverSlop / 2, volume.height / 2)
        wait(50)
        compare(volume.open, true)
        mouseMove(volume, volume.width / 2, -volume.height)
        wait(50)
        compare(volume.open, true)
        const panelTop = -Theme.audioOutput.volumePanelHeight
        mouseMove(volume, volume.width / 2, panelTop - Theme.audioOutput.volumeHoverSlop / 2)
        wait(50)
        compare(volume.open, true)
        mouseMove(volume, volume.width / 2, panelTop - 5 * Theme.audioOutput.volumeHoverSlop)
        tryCompare(volume, "open", false)
    }
    function test_volume_folds_after_drag_outside() {
        mouseMove(volume)
        tryCompare(volume, "open", true)
        waitForRendering(volume)
        const slider = findChild(volume, "volumeVerticalSlider")
        verify(slider !== null)
        mousePress(slider, slider.width / 2, slider.height / 2)
        mouseMove(slider, slider.width / 2 - 200, slider.height / 2 - 30)
        compare(volume.open, true)
        mouseRelease(slider, slider.width / 2 - 200, slider.height / 2 - 30)
        mouseMove(elsewhere)
        tryCompare(volume, "open", false)   // even with the slider still holding focus
    }
    function test_volume_folds_on_press_outside() {
        volume.showSlider()
        tryCompare(volume, "open", true)
        wait(100)
        const before = target.hits
        mouseClick(target)
        tryCompare(volume, "open", false)
        compare(target.hits, before + 1)    // the press still reached what was under it
    }

    function test_volume_folds_on_escape() {
        volume.showSlider()
        tryCompare(volume, "open", true)
        keyClick(Qt.Key_Escape)
        tryCompare(volume, "open", false)
    }
    function test_volume_reopens_after_press_outside() {
        volume.showSlider()
        tryCompare(volume, "open", true)
        wait(100)
        mouseClick(target)
        tryCompare(volume, "open", false)
        wait(100)
        mouseMove(volume)                   // a fresh hover opens it again
        tryCompare(volume, "open", true)
        const slider = findChild(volume, "volumeVerticalSlider")
        tryVerify(function () { return slider.visible && slider.width > 0 }, 1000, "panel shown again")
        mouseMove(elsewhere)
        tryCompare(volume, "open", false)
    }

    function test_window_buttons_use_native_commands() {
        Backend.Desktop.lastWindowCommand = ""
        caption.trigger("minimize")
        compare(Backend.Desktop.lastWindowCommand, "minimize")
        caption.trigger("zoom")
        compare(Backend.Desktop.lastWindowCommand, "toggleMaximize")
    }

    function test_band_contained() {
        for (const p of wave.wavePoints) {
            verify(p.y >= wave.outlineWidth - 1e-6 && p.y <= wave.height + 1e-6, "wave y " + p.y)
            verify(p.x >= 0 && p.x <= wave.width)
        }
    }
    function test_band_glides_between_slow_updates() {
        wave.bands = test.loud(0.2)
        wait(120)
        wave.bands = test.loud(0.5)         // 120 ms apart: slower than any screen
        verify(wave.glide < 1)
        tryCompare(wave, "glide", 1, 500)
        compare(wave.drawn, wave.bands)
    }

    function test_resize_handle() {
        mouseDrag(sizer, sizer.width / 2, 50, 100, 0)
        tryVerify(function () { return panel.width > 300 })
        mouseDrag(sizer, sizer.width / 2, 50, 900, 0)
        compare(panel.width, 420)
        mouseDoubleClickSequence(sizer, sizer.width / 2, 50)
        compare(panel.width, 232)
    }
}
