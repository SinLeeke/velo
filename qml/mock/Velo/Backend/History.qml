pragma Singleton
import QtQuick

// MOCK of Velo.Backend.History. Preview only: 120 sample entries,
// the first batch is 50 and loadMore() appends 50 more after a simulated 300 ms. The
// single-shot Timer exists only in this mock and only while a batch is "loading".
QtObject {
    id: history

    readonly property int total: 120
    readonly property int batch: 50
    property ListModel model: ListModel {}
    property bool loading: false
    readonly property bool hasMore: model.count < total && !emptyDemo
    property string error: ""
    readonly property int count: emptyDemo ? 0 : total
    property bool loadedOnce: false
    // PREVIEW ONLY: empties the list (and the total) to look at the empty state.
    property bool emptyDemo: false
    property int loadCalls: 0          // PREVIEW: how many times load() reached the backend
    property int loadMoreCalls: 0      // PREVIEW: how many loadMore() actually started a batch

    signal changed()
    signal cleared()

    readonly property var titles: ["Marea de cobalto", "Faro sin costa", "Insomne", "Sal en la ventana", "La última luz", "Un lugar al que volver", "Agua quieta", "Cuando amanezca"]
    readonly property var artists: ["Lumen Fiel", "Bosque Bajo, Iria Sol", "Iria Sol", "Lumen Fiel"]

    function appendBatch(n) {
        const now = Date.now()
        const from = model.count
        for (let i = from; i < Math.min(total, from + n); ++i) {
            // Newest first: 4 min apart at the top, drifting to days further down.
            const ago = i < 10 ? i * 4 * 60000 : i < 30 ? (i - 8) * 25 * 60000 : (i - 25) * 7 * 3600000
            model.append({
                entryId: "h" + i, uri: "spotify:track:h" + i,
                title: titles[i % titles.length] + (i >= 8 ? " " + (Math.floor(i / 8) + 1) : ""),
                artist: artists[i % artists.length], album: "Atlas de agua",
                imageUrl: "", artistId: "a" + (i % 4), albumId: "al" + (i % 3),
                contextUri: "", durationMs: 180000 + (i % 7) * 13000, playedAtMs: now - ago
            })
        }
    }
    function load() {
        loadCalls += 1
        if (loadedOnce || loading) return
        loadedOnce = true
        if (!emptyDemo) appendBatch(batch)
    }
    function loadMore() {
        if (loading || !hasMore) return
        loadMoreCalls += 1
        loading = true
        moreTimer.restart()
    }
    function playEntry(entryId) {
        for (let i = 0; i < model.count; ++i) {
            if (model.get(i).entryId === entryId) {
                Player.title = model.get(i).title
                Player.artist = model.get(i).artist
                return
            }
        }
    }
    function clear() { model.clear(); cleared() }
    function showEmpty() { model.clear(); emptyDemo = true; loadedOnce = true }

    property Timer moreTimer: Timer {
        interval: 300
        onTriggered: { history.appendBatch(history.batch); history.loading = false; history.changed() }
    }
}
