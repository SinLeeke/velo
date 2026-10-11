import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../components/library"

// v0.9.4 (DJ): in the library list (Recientes order) DJ Livi's playlist is not a list to open:
// it shows as «DJ Livi», starts the DJ on click and during a session it is «Cambiar de onda».
TestCase {
    id: test
    name: "V94Dj"
    when: windowShown
    visible: true
    width: 400
    height: 600

    Component { id: sidebarComponent; LibrarySidebar { width: Theme.size.sidebar; height: test.height } }
    property var saved: []
    function init() {
        Theme.reduceMotion = true
        Backend.Account.signedIn = true
        Backend.Account.isPremium = true
        Backend.Account.audioReady = true
        Backend.Player.djAvailable = true
        Backend.Player.djActive = false
        Backend.Library.sidebarStatus = "ready"
        saved = []
        for (let i = 0; i < Backend.Library.sidebarItems.count; ++i) {
            const row = Backend.Library.sidebarItems.get(i), copy = {}
            for (const key in row) copy[key] = row[key]
            saved.push(copy)
        }
        Backend.Library.sidebarItems.clear()
    }
    function cleanup() {
        Backend.Library.sidebarItems.clear()
        for (const row of saved) Backend.Library.sidebarItems.append(row)
        Backend.Player.djActive = false
    }
    function row(uri, name) {
        return { kind: "playlist", objectId: uri.split(":")[2], uri: uri, name: name, creator: "Spotify",
                 imageUrl: "", pinned: false, owned: false, addedAtMs: -1, lastPlayedAtMs: 1 }
    }
    function test_dj_row_starts_the_dj_instead_of_opening_a_playlist() {
        Backend.Library.sidebarItems.append(row("spotify:playlist:37i9dQZF1EYkqdzj48dyYq", "DJ"))
        Backend.Library.sidebarItems.append(row("spotify:playlist:p1", "Otra"))
        const sidebar = createTemporaryObject(sidebarComponent, test)
        verify(sidebar)
        const spy = createTemporaryObject(spyComponent, test, { target: sidebar, signalName: "navigate" })
        tryVerify(() => findChild(sidebar, "sidebarEntry0") !== null)
        const dj = findChild(sidebar, "sidebarEntry0")
        verify(dj.isDj)
        compare(dj.shownName, qsTr("DJ Livi"))
        verify(!findChild(sidebar, "sidebarEntry1").isDj)
        mouseClick(dj)
        verify(Backend.Player.djActive)
        compare(spy.count, 0)
        mouseClick(findChild(sidebar, "sidebarEntry1"))
        compare(spy.count, 1)
    }
    Component { id: spyComponent; SignalSpy {} }
}
