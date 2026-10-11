import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../views" as Views

// G2 (A-LB1): activating audio fades the notice out and the page glides up instead of jumping.
TestCase {
    id: test
    name: "G2Notice"
    when: windowShown
    visible: true
    width: 1100
    height: 760
    Component { id: libraryComponent; Views.Library { width: test.width; height: test.height } }

    function init() { Theme.performanceMode = false; Backend.Account.signedIn = true; Backend.Account.audioReady = false }
    function cleanup() { Theme.performanceMode = false; Backend.Account.audioReady = true }

    function test_notice_leaves_with_fade_and_page_glides() {
        const lib = createTemporaryObject(libraryComponent, test)
        verify(lib)
        const notice = findChild(lib, "audioSetupNotice")
        verify(notice.active)
        const pageLoader = lib.currentPageItem.parent
        wait(50)
        Backend.Account.audioReady = true
        verify(notice.active)              // still there while it fades
        tryCompare(notice, "active", false, 2000)
        // after the exit the space is released and the page starts displaced, then settles
        tryCompare(pageLoader.transform[0], "y", 0, 2000)
        compare(lib.noticeOn, false)
    }

    function test_notice_is_removed_at_once_in_performance_mode() {
        Theme.performanceMode = true
        const lib = createTemporaryObject(libraryComponent, test)
        const notice = findChild(lib, "audioSetupNotice")
        verify(notice.active)
        Backend.Account.audioReady = true
        compare(notice.active, false)
        compare(lib.noticeOn, false)
    }

    function test_notice_returns_if_audio_is_turned_off_mid_exit() {
        const lib = createTemporaryObject(libraryComponent, test)
        const notice = findChild(lib, "audioSetupNotice")
        Backend.Account.audioReady = true
        Backend.Account.audioReady = false
        verify(notice.active)
        compare(notice.opacity, 1)
    }
}
