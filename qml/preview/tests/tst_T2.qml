import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/window"
import "../../theme"

TestCase {
    id: test
    name: "T2"
    visible: true
    width: 600
    height: 200
    when: windowShown

    TitleBar { id: bar; width: 600; height: 40; controlsStyle: "windows" }

    function indicator() { return findChild(bar, "") }
    function texts(item, out) {
        for (let i = 0; i < item.children.length; ++i) {
            const c = item.children[i]
            if (c.text !== undefined && c.font !== undefined) out.push(c)
            texts(c, out)
        }
        return out
    }
    function row() {
        for (let i = 0; i < bar.children.length; ++i)
            if (bar.children[i].remainingMin !== undefined) return bar.children[i]
        return null
    }
    function timerOf(r) {
        for (let i = 0; i < r.data.length; ++i)
            if (r.data[i].interval !== undefined) return r.data[i]
        return null
    }
    function cleanup() {
        Backend.Actions.sleepMinutes = 0
        Backend.Actions.sleepEndsAtMs = 0
        Theme.playing = true
        Theme.windowVisible = true
    }
    function test_hiddenWhenOff() {
        Backend.Actions.sleepMinutes = 0
        verify(!row().visible)
    }
    function test_endOfTrack() {
        Backend.Actions.sleepMinutes = -1
        verify(row().visible)
        compare(texts(row(), [])[0].text, qsTr("Al terminar la canción"))
    }
    function test_minutesRoundUp() {
        Backend.Actions.sleepMinutes = 30
        Backend.Actions.sleepEndsAtMs = Date.now() + 23.5 * 60000
        verify(row().visible)
        compare(texts(row(), [])[0].text, qsTr("%1 min").arg(24))
    }
    function test_timerRunsOnlyWhenPlayingAndVisible() {
        Backend.Actions.sleepMinutes = 30
        Backend.Actions.sleepEndsAtMs = Date.now() + 10 * 60000
        const t = timerOf(row())
        verify(t.running)
        Theme.playing = false
        verify(!t.running)
        Theme.playing = true
        Theme.windowVisible = false
        verify(!t.running)
        Theme.windowVisible = true
        verify(t.running)
    }
}
