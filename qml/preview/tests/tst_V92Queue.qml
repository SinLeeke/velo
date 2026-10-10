import QtQuick
import QtTest
import Velo.Backend
import "../../components/queue"
import "../../theme"

TestCase {
    id: test
    name: "V92Queue"
    visible: true
    width: 900
    height: 700
    when: windowShown
    Component { id: panelComponent; QueuePanel {} }
    property var saved
    function initTestCase() {
        const rows = []
        for (let i = 0; i < History.model.count; ++i) rows.push(Object.assign({}, History.model.get(i)))
        saved = { rows: rows, loading: History.loading, error: History.error,
            emptyDemo: History.emptyDemo, loadedOnce: History.loadedOnce,
            title: Player.title, artist: Player.artist, queueStatus: Player.queueStatus,
            queueError: Player.queueError, reduceMotion: Theme.reduceMotion }
    }
    function init() {
        Theme.reduceMotion = true
        History.moreTimer.stop()
        History.model.clear()
        History.emptyDemo = true
        History.loadedOnce = true
        History.loading = false
        History.error = ""
    }
    function cleanupTestCase() {
        History.moreTimer.stop()
        History.model.clear()
        saved.rows.forEach(row => History.model.append(row))
        History.loading = saved.loading
        History.error = saved.error
        History.emptyDemo = saved.emptyDemo
        History.loadedOnce = saved.loadedOnce
        Player.title = saved.title
        Player.artist = saved.artist
        Player.queueStatus = saved.queueStatus
        Player.queueError = saved.queueError
        Theme.reduceMotion = saved.reduceMotion
    }
    function panel() {
        const p = createTemporaryObject(panelComponent, test)
        verify(p !== null)
        p.open()
        p.selectTab(1)
        wait(0)
        return p
    }
    function test_initial_history_loading_error_and_empty_are_visible() {
        const p = panel()
        const status = findChild(p, "recentsEmpty")
        History.loading = true
        tryCompare(status, "visible", true)
        compare(status.text, qsTr("Cargando…"))
        History.loading = false
        History.error = "Fixture: error de lectura"
        compare(status.text, qsTr("No se pudo cargar lo que escuchaste. Vuelve a abrir el panel para reintentar."))
        History.error = ""
        compare(status.text, qsTr("Lo que escuches con Velo aparecerá aquí."))
    }
    function test_history_selected_row_plays_with_keyboard_and_click() {
        const p = panel()
        for (let i = 0; i < 2; ++i) History.model.append({
            entryId: "v92-" + i, uri: "spotify:track:" + i,
            title: "Fila " + i, artist: "Artista", album: "Álbum", imageUrl: "",
            artistId: "", albumId: "", contextUri: "", durationMs: 100000, playedAtMs: Date.now()
        })
        const list = findChild(p, "recentsList")
        tryCompare(list, "count", 2)
        list.currentIndex = 1
        list.forceActiveFocus()
        keyClick(Qt.Key_Return)
        compare(Player.title, "Fila 1")
        Player.title = "Otro"
        keyClick(Qt.Key_Space)
        compare(Player.title, "Fila 1")
        list.currentIndex = 0
        mouseClick(list.currentItem, list.width / 2, list.currentItem.height / 2)
        compare(Player.title, "Fila 0")
        verify(list.reuseItems)
    }
    function test_queue_error_is_not_an_empty_queue() {
        const p = panel()
        p.selectTab(0)
        Player.queueStatus = "unavailable"
        Player.queueError = "Fixture: cola no disponible"
        const status = findChild(p, "queueEmptyText")
        tryCompare(status, "visible", true)
        compare(status.text, qsTr("No se pudo cargar la cola. Vuelve a abrir el panel para reintentar."))
        verify(findChild(p, "queueList").reuseItems)
    }
}
