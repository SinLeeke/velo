import QtQuick
import QtTest
import QtCore
import Velo.Backend as Backend
import "../../components/library"
import "../../theme"

TestCase {
    id: test
    name: "AudioControls"
    visible: true
    width: 960
    height: 640
    when: windowShown

    Rectangle { anchors.fill: parent; color: Theme.bg }
    VolumeButton { id: volume; x: test.width - width * 3; y: test.height - height }
    AudioOutputButton { id: output; x: test.width - width * 2; y: test.height - height }
    Item { id: elsewhere; width: 20; height: 20; focus: true }
    HomePage { id: home; x: 40; y: 40; width: test.width - 80; height: test.height - 120; visible: false }
    // Mirrors Main.qml's window shortcut to check focused controls consume Space.
    Shortcut { sequence: "Space"; onActivated: Backend.Player.playPause() }

    function capture(name) {
        const temp = decodeURIComponent(StandardPaths.writableLocation(StandardPaths.TempLocation).toString())
            .replace(/^file:\/\//, "").replace(/^\/(?=[A-Za-z]:)/, "")
        grabImage(test.Window.window.contentItem).save(temp + "/velo-u3-" + name + ".png")
    }

    function init() {
        Backend.Player.volumeLatencyMs = 0
        home.visible = false
        output.outputPopup.close()
        elsewhere.forceActiveFocus()
        mouseMove(elsewhere)
        Backend.Player.canControl = true
        Backend.Player.volume = 72
        Backend.Player.isPlaying = true
        Backend.Player.setOutput("output-speakers")
        volume.dismissed = false
    }

    function test_home_first_card_hover_is_inside_vertical_clip() {
        home.visible = true
        const viewport = findChild(home, "homeCardViewport")
        const card = findChild(home, "homeFirstPlaylistCard")
        verify(card !== null)
        card.forceActiveFocus()
        elsewhere.forceActiveFocus()
        mouseMove(card)
        tryCompare(card, "hovered", true)
        verify(!card.visualFocus)
        const at = card.mapToItem(viewport, 0, 0)
        verify(viewport.clip)
        verify(at.x >= 0 && at.x + card.width <= viewport.width)
        for (const mode of ["dark", "light"]) {
            Theme.mode = mode
            waitForRendering(card)
            capture(mode + "-first-card-hover")
        }
        Backend.Player.volumeLatencyMs = 0
        home.visible = false
    }

    function test_volume_and_output_open_together() {
        output.outputPopup.open()
        tryCompare(output.outputPopup, "opened", true)
        mouseMove(volume)
        tryCompare(volume, "open", true)
        for (const mode of ["dark", "light"]) {
            Theme.mode = mode
            waitForRendering(output)
            capture(mode + "-volume-and-output")
        }
        output.outputPopup.close()
    }

    function test_volume_hover_click_and_focus() {
        mouseMove(volume)
        tryCompare(volume, "open", true)
        mouseClick(volume)
        tryCompare(Backend.Player, "volume", 0)
        tryCompare(Backend.Player, "lastAudibleVolume", 72)
        mouseClick(volume)
        tryCompare(Backend.Player, "volume", 72)
        mouseMove(elsewhere)
        elsewhere.forceActiveFocus()
        tryCompare(volume, "open", false)
        volume.showSlider()
        tryCompare(volume, "open", true)
        keyClick(Qt.Key_Escape)
        compare(volume.open, false)
        keyClick(Qt.Key_Up)
        compare(volume.open, true)
        tryCompare(Backend.Player, "volume", 77)
        keyClick(Qt.Key_Down)
        tryCompare(Backend.Player, "volume", 72)
        findChild(volume, "volumeVerticalSlider").forceActiveFocus()
        keyClick(Qt.Key_Escape)
        compare(volume.open, false)
    }

    function test_volume_keyboard_and_external_memory() {
        Backend.Player.volume = 29 // another view, shortcut or authoritative source
        volume.showSlider()
        keyClick(Qt.Key_Space)
        tryCompare(Backend.Player, "volume", 0)
        compare(Backend.Player.isPlaying, true)
        keyClick(Qt.Key_Return)
        tryCompare(Backend.Player, "volume", 29)
        Backend.Player.volume = 83
        Backend.Player.volume = 0
        keyClick(Qt.Key_Enter)
        tryCompare(Backend.Player, "volume", 83)
        compare(findChild(volume, "volumeMuteToggle").text, qsTr("Silenciar"))
        Backend.Player.canControl = false
        verify(!volume.open)
        keyClick(Qt.Key_Return)
        tryCompare(Backend.Player, "volume", 83)
    }

    function test_output_select_keyboard_and_external_update() {
        Backend.Player.canControl = false // the system selector needs no Spotify session
        mouseClick(output)
        tryCompare(output.outputPopup, "opened", true)
        const list = findChild(output.outputPopup.contentItem, "systemAudioOutputList")
        verify(list !== null)
        compare(list.currentIndex, 0)
        keyClick(Qt.Key_Down)
        keyClick(Qt.Key_Space)
        tryCompare(output.outputPopup, "opened", false)
        compare(Backend.Player.outputs.get(1).isDefault, true)
        compare(Backend.Player.isPlaying, true)
        output.outputPopup.open()
        tryCompare(output.outputPopup, "opened", true)
        compare(list.currentIndex, 1)
        Backend.Player.setOutput("output-monitor")
        compare(findChild(list, "systemAudioOutputRow2").isDefault, true)
        keyClick(Qt.Key_Escape)
        tryCompare(output.outputPopup, "opened", false)
        compare(output.activeFocus, true)
    }
}
