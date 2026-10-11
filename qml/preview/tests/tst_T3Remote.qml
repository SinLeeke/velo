import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views

// T3: «Reproduciendo en <dispositivo>» strip: shows the active device while another one plays,
// disappears (fade) when playback returns to this PC, lifts the player bar by its height.
TestCase {
    id: test
    name: "T3Remote"
    when: windowShown
    visible: true
    width: 1300
    height: 800
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }

    function init() {
        Theme.reduceMotion = true
        Backend.Account.signedIn = true
        Backend.Account.audioReady = true
        Backend.Player.hasSession = true
        Backend.Player.remote = false
    }
    function cleanup() { Backend.Player.remote = false; Backend.Player.deviceIcon = "computer" }

    function test_shows_only_with_another_device() {
        const lib = createTemporaryObject(libraryComponent, test)
        const strip = lib.remoteStrip
        compare(strip.visible, false)
        compare(lib.bottomReserve, Theme.size.playerBar)
        Backend.Player.deviceName = "Altavoz salón"
        Backend.Player.deviceIcon = "speaker"
        Backend.Player.remote = true
        tryCompare(strip, "visible", true)
        compare(findChild(strip, "remoteDeviceText").text, qsTr("Reproduciendo en %1").arg("Altavoz salón"))
        compare(strip.height, Theme.remoteStrip.height)
        compare(lib.bottomReserve, Theme.size.playerBar + Theme.remoteStrip.height)
        const bar = findChild(lib, "libraryPlayerBar")
        compare(bar.mapToItem(lib, 0, bar.height).y, lib.height - Theme.remoteStrip.height)
        Backend.Player.remote = false
        tryCompare(strip, "visible", false)
        compare(lib.bottomReserve, Theme.size.playerBar)
        compare(bar.mapToItem(lib, 0, bar.height).y, lib.height)
    }

    function test_hidden_without_session() {
        const lib = createTemporaryObject(libraryComponent, test)
        Backend.Player.remote = true
        Backend.Player.hasSession = false
        tryCompare(lib.remoteStrip, "visible", false)
        Backend.Player.hasSession = true
    }
}
