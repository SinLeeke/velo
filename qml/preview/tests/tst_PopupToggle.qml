import QtQuick
import QtQuick.Templates as T
import QtTest
import Velo.Backend
import "../../views" as Views
import "../../components/library"
import "../../theme"

// v0.9.2: pressing a popup's own button again closes it (the press used to close it as an
// outside press and the click reopened it). Volume is left alone (a second press mutes).
TestCase {
    id: test
    name: "PopupToggle"
    visible: true
    width: 1100
    height: 680
    when: windowShown

    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar; y: test.height - height } }
    Component { id: npComponent; Views.NowPlaying { width: test.width; height: 600; ownsCredits: false } }
    Component {
        id: outsideComponent
        T.Popup {
            x: 10; y: 10; width: 80; height: 40
            closePolicy: T.Popup.CloseOnEscape | T.Popup.CloseOnPressOutside
        }
    }
    AudioOutputButton { id: output; x: test.width / 2; y: test.height / 2 }
    SignalSpy { id: devicesSpy; signalName: "devicesRequested" }

    function init() {
        Theme.reduceMotion = true
        Player.hasSession = true
        Player.canControl = true
        Player.isPlaying = true
        output.outputPopup.close()
    }
    function make(component) {
        const item = createTemporaryObject(component, test)
        verify(item !== null)
        wait(0)
        return item
    }

    function test_audio_output_toggles_and_closes_outside() {
        mouseClick(output)
        tryCompare(output.outputPopup, "opened", true)
        mouseClick(output)
        tryCompare(output.outputPopup, "opened", false)
        mouseClick(output)
        tryCompare(output.outputPopup, "opened", true)
        mouseClick(test, 5, test.height - 5)
        tryCompare(output.outputPopup, "opened", false)
    }
    function test_bar_visualizer_menu_toggles_and_closes_outside() {
        const bar = make(barComponent)
        const button = findChild(bar, "playerBarVisualizerButton")
        const menu = findChild(bar, "playerBarVizMenu")
        mouseClick(button)
        tryCompare(menu, "opened", true)
        mouseClick(button)
        tryCompare(menu, "opened", false)
        mouseClick(button)
        tryCompare(menu, "opened", true)
        mouseClick(test, 5, 5)
        tryCompare(menu, "opened", false)
    }
    function test_bar_devices_click_that_closes_a_popup_does_not_request() {
        const bar = make(barComponent)
        devicesSpy.target = bar
        devicesSpy.clear()
        const button = findChild(bar, "playerBarDevices")
        mouseClick(button)
        compare(devicesSpy.count, 1)
        // Stand-in for the open device picker: this press only closes it.
        const picker = createTemporaryObject(outsideComponent, test)
        picker.open()
        tryCompare(picker, "opened", true)
        mouseClick(button)
        tryCompare(picker, "opened", false)
        compare(devicesSpy.count, 1)
    }
    function test_now_playing_devices_toggles_and_closes_outside() {
        const np = make(npComponent)
        const button = findChild(np, "npDevicesButton")
        const picker = findChild(np, "npDevicePicker")
        mouseClick(button)
        tryCompare(picker, "opened", true)
        mouseClick(button)
        tryCompare(picker, "opened", false)
        wait(50)
        compare(picker.opened, false)
        mouseClick(button)
        tryCompare(picker, "opened", true)
        mouseClick(np, np.width / 3, np.height - 20)
        tryCompare(picker, "opened", false)
    }
}
