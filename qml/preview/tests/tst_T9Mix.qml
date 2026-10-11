import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../components/library"

// T9: «Mixear» (crossfade between songs). The playlist header button is a toggle that needs
// gapless playback in Velo's own engine; Settings carries the auto switch and the seconds.
TestCase {
    id: test
    name: "T9Mix"
    when: windowShown
    visible: true
    width: 900
    height: 700

    Component { id: tracksComponent; TrackListPage { width: test.width; height: test.height; playlistId: "t9-list" } }
    Component { id: settingsComponent; SettingsPage { width: test.width; height: test.height } }

    function init() {
        Theme.reduceMotion = true
        Backend.Player.canControl = true
        Backend.Player.djActive = false
        Backend.Library.currentKind = "playlist"
        Backend.Library.currentListable = true
        Backend.Library.mixCurrent = false
        Backend.Settings.gapless = true
        Backend.Settings.mixAuto = false
        Backend.Settings.mixSeconds = 6
    }
    function cleanup() {
        Backend.Settings.gapless = true
        Backend.Library.mixCurrent = false
    }

    function test_button_states_and_toggle() {
        const page = createTemporaryObject(tracksComponent, test)
        verify(page !== null)
        const button = findChild(page, "mixButton")
        verify(button !== null)
        verify(button.visible)
        verify(button.enabled)
        verify(!button.active)
        compare(button.text, qsTr("Activar Mixear"))
        mouseClick(button)
        verify(Backend.Library.mixCurrent)
        verify(button.active)
        verify(button.toggle)
        compare(button.text, qsTr("Desactivar Mixear"))
        mouseClick(button)
        verify(!Backend.Library.mixCurrent)
        compare(button.text, qsTr("Activar Mixear"))
    }

    function test_button_is_disabled_without_gapless_or_with_dj() {
        const page = createTemporaryObject(tracksComponent, test)
        const button = findChild(page, "mixButton")
        Backend.Settings.gapless = false
        verify(!Backend.Player.mixAvailable)
        verify(!button.enabled)
        compare(button.text, qsTr("Requiere reproducción sin pausas"))
        mouseClick(button)
        verify(!Backend.Library.mixCurrent)
        Backend.Settings.gapless = true
        Backend.Player.djActive = true
        verify(!button.enabled)
        Backend.Player.djActive = false
        verify(button.enabled)
    }

    function test_button_hidden_in_albums_and_shown_in_liked_songs() {
        const page = createTemporaryObject(tracksComponent, test)
        Backend.Library.currentKind = "album"
        verify(!findChild(page, "mixButton").visible)
        Backend.Library.currentKind = "liked"
        verify(findChild(page, "mixButton").visible)
    }

    function test_settings_rows_follow_gapless() {
        const page = createTemporaryObject(settingsComponent, test)
        verify(page !== null)
        const toggle = findChild(page, "mixAutoToggle")
        const slider = findChild(page, "mixSecondsSlider")
        verify(toggle !== null && slider !== null)
        verify(toggle.enabled && slider.enabled)
        compare(slider.from, 1)
        compare(slider.to, 12)
        compare(slider.value, 6)
        Backend.Settings.mixSeconds = 9
        compare(slider.value, 9)
        Backend.Settings.mixAuto = true
        verify(toggle.checked)
        Backend.Settings.gapless = false
        verify(!toggle.enabled && !slider.enabled)
    }
}
