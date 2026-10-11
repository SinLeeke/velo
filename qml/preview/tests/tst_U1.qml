import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components"
import "../../components/library"
import "../../theme"

TestCase {
    id: test
    name: "U1"
    width: 1100; height: 720
    visible: true
    when: windowShown
    Component { id: noticeComponent; UpdateNotice {} }
    Component { id: settingsComponent; SettingsPage { width: 900; height: 650; focusSection: "about" } }
    function init() {
        Theme.reduceMotion = true
        Backend.Updater.state = "idle"
        Backend.Updater.error = ""
        Backend.Updater.latestVersion = "1.0.0"
        Backend.Updater.portable = false
        Backend.Updater.dismissCalls = 0
        Backend.Updater.installCalls = 0
        Backend.Updater.openReleasePageCalls = 0
        Backend.Updater.checkCalls = 0
        Backend.Updater.checkResult = "idle"
        Backend.Updater.dismissed = false
        Backend.Settings.checkUpdates = true
    }
    function cleanup() { Backend.Updater.state = "idle"; Backend.Settings.checkUpdates = true }
    function make(component) {
        const obj = createTemporaryObject(component, test)
        verify(obj !== null)
        return obj
    }
    function test_no_new_version_is_silent() {
        const notice = make(noticeComponent)
        Backend.Updater.checkNow()
        compare(notice.visible, false)
    }
    function test_available_and_dismiss() {
        const notice = make(noticeComponent)
        Backend.Updater.state = "available"
        tryCompare(notice, "visible", true)
        compare(findChild(notice, "updateMessage").text, "Velo 1.0.0 está listo")
        const update = findChild(notice, "updateInstall")
        const later = findChild(notice, "updateDismiss")
        verify(update.visible); compare(update.text, "Actualizar")
        verify(later.visible); compare(later.text, "Más tarde")
        waitForRendering(notice)
        mouseClick(later)
        compare(Backend.Updater.dismissCalls, 1)
        compare(notice.visible, false)
        Backend.Updater.checkResult = "available"
        Backend.Updater.checkNow()
        compare(notice.visible, false)
    }
    function test_install_and_progress() {
        const notice = make(noticeComponent)
        Backend.Updater.state = "available"
        waitForRendering(notice)
        mouseClick(findChild(notice, "updateInstall"))
        compare(Backend.Updater.installCalls, 1)
        Backend.Updater.progress = 0.42
        compare(findChild(notice, "updateMessage").text, "Descargando Velo " + Backend.Updater.latestVersion)
        verify(!findChild(notice, "updateDismiss").visible)
    }
    function test_portable_opens_release() {
        const notice = make(noticeComponent)
        Backend.Updater.portable = true
        Backend.Updater.state = "available"
        compare(findChild(notice, "updateInstall").text, "Actualizar")
        waitForRendering(notice)
        mouseClick(findChild(notice, "updateInstall"))
        compare(Backend.Updater.openReleasePageCalls, 1)
    }
    function test_error_retry() {
        const notice = make(noticeComponent)
        Backend.Updater.state = "error"
        Backend.Updater.error = "La descarga está incompleta. Inténtalo de nuevo."
        compare(findChild(notice, "updateMessage").text, "No se pudo actualizar")
        const retry = findChild(notice, "updateInstall")
        compare(retry.text, "Reintentar")
        waitForRendering(notice)
        mouseClick(retry)
        compare(Backend.Updater.installCalls, 1)
    }
    function test_settings_toggle_writes_preference() {
        const page = make(settingsComponent)
        const toggle = findChild(page, "settingsCheckUpdates")
        verify(toggle !== null)
        tryVerify(() => toggle.visible && toggle.mapToItem(test, 0, 0).y < test.height)
        compare(toggle.checked, true)
        mouseClick(toggle)
        compare(Backend.Settings.checkUpdates, false)
    }
    function test_manual_check_result() {
        const page = make(settingsComponent)
        const button = findChild(page, "settingsCheckNow")
        verify(button !== null)
        tryVerify(() => button.visible && button.mapToItem(test, 0, 0).y < test.height)
        mouseClick(button)
        compare(Backend.Updater.checkCalls, 1)
        verify(button.enabled)
        compare(findChild(page, "settingsUpdateResult").hint, "Tienes la última versión")
        Backend.Updater.checkResult = "available"
        mouseClick(button)
        compare(Backend.Updater.checkCalls, 2)
        compare(findChild(page, "settingsUpdateResult").hint, "Velo 1.0.0 disponible")
    }
}
