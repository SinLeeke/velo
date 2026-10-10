import QtQuick
import QtTest
import Velo.Backend
import "../../components/devices"
import "../../theme"

TestCase {
    id: test
    name: "V92dev"
    visible: true
    width: 640
    height: 760
    when: windowShown

    Component { id: pickerComponent; DevicePicker {} }
    property var savedDevices: []
    property bool savedAutoComplete
    property bool savedMotion
    property int savedRefreshCount
    function initTestCase() {
        for (let i = 0; i < Player.devices.count; ++i) {
            const d = Player.devices.get(i)
            savedDevices.push({deviceId:d.deviceId, name:d.name, kind:d.kind, isActive:d.isActive})
        }
        savedAutoComplete = Player.devicesAutoComplete
        savedRefreshCount = Player.devicesRefreshCount
        savedMotion = Theme.reduceMotion
    }
    function init() {
        Player.devices.clear()
        Player.devicesAutoComplete = false
        Player.devicesRefreshCount = 0
        Theme.reduceMotion = true
    }
    function cleanupTestCase() {
        Player.devices.clear()
        for (const d of savedDevices) Player.devices.append(d)
        Player.devicesAutoComplete = savedAutoComplete
        Player.devicesRefreshCount = savedRefreshCount
        Theme.reduceMotion = savedMotion
    }
    function openPicker() {
        const picker = createTemporaryObject(pickerComponent, test)
        verify(picker !== null)
        picker.open()
        tryCompare(picker, "opened", true)
        return picker
    }
    function test_empty_to_empty_finishes_and_reopens() {
        const picker = openPicker()
        compare(picker.loading, true)
        Player.devicesRefreshed("") // No model reset/countChanged: the original stuck wait.
        compare(picker.loading, false)
        verify(findChild(picker.contentItem, "devicesStatus").visible)
        verify(findChild(picker.contentItem, "devicesStatusText").text.indexOf("celular") >= 0)
        picker.close()
        tryCompare(picker, "opened", false)
        picker.open()
        tryCompare(picker, "opened", true)
        compare(Player.devicesRefreshCount, 2)
        compare(picker.loading, true)
        picker.close()
    }
    function test_errors_finish_and_retry_data() {
        return ["rateLimited", "auth", "forbidden", "network"].map(code => ({tag:code, code:code}))
    }
    function test_errors_finish_and_retry(data) {
        const picker = openPicker()
        Player.error("unrelated error")
        compare(picker.loading, true)
        Player.devicesRefreshed(data.code)
        compare(picker.loading, false)
        compare(picker.errorCode, data.code)
        const retry = findChild(picker.contentItem, "devicesRetry")
        compare(retry.text, "Reintentar")
        verify(retry.activeFocus)
        keyClick(Qt.Key_Return)
        compare(Player.devicesRefreshCount, 2)
        compare(picker.loading, true)
        compare(picker.errorCode, "")
        Player.devices.append({deviceId:"phone", name:"Pixel", kind:"Smartphone", isActive:true})
        Player.devices.append({deviceId:"pc", name:"Velo", kind:"Computer", isActive:false})
        Player.devicesRefreshed("")
        compare(picker.refreshing, false)
        compare(picker.iconFor("Smartphone"), "smartphone")
        const list = picker.contentItem.children.find(child => child.model === Player.devices)
        verify(list !== undefined)
        tryCompare(list, "count", 2)
        verify(list.activeFocus)
        tryVerify(() => list.itemAtIndex(0) !== null)
        compare(list.itemAtIndex(0).text, "Pixel")
        compare(list.itemAtIndex(0).active, true)
        verify(!findChild(picker.contentItem, "devicesSkeleton").visible)
        verify(!findChild(picker.contentItem, "devicesStatus").visible)
        picker.close()
    }
    function test_cached_rows_do_not_hide_error_or_skip_refresh() {
        Player.devices.append({deviceId:"phone", name:"Pixel", kind:"Smartphone", isActive:true})
        const picker = openPicker()
        compare(Player.devicesRefreshCount, 1)
        Player.devicesRefreshed("rateLimited")
        verify(findChild(picker.contentItem, "devicesStatus").visible)
        compare(Player.devices.count, 1)
        picker.close()
    }
    function test_synchronous_completion() {
        Player.devicesAutoComplete = true
        const picker = openPicker()
        compare(picker.refreshing, false)
        compare(picker.loading, false)
        picker.close()
    }
    function test_null_id_remains_visible_without_transferring() {
        Player.devices.append({deviceId:"", name:"Restricted phone", kind:"Smartphone", isActive:true})
        const picker = openPicker()
        Player.devicesRefreshed("")
        const list = picker.contentItem.children.find(child => child.model === Player.devices)
        tryVerify(() => list.itemAtIndex(0) !== null)
        compare(list.itemAtIndex(0).text, "Restricted phone")
        list.itemAtIndex(0).activate()
        compare(picker.opened, true)
        picker.close()
    }
}
