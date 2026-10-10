pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Player. Preview only: no timers, positionMs never
// advances on its own (the real bridge pushes it at <= 4 Hz while playing and visible).
QtObject {
    id: player

    property string title: "Marea de cobalto (versión extendida en directo)"
    property string artist: "Lumen Fiel"
    property string album: "Atlas de agua"
    property string trackUri: "spotify:track:mock"
    property url coverUrl: Qt.resolvedUrl("../../assets/cover_teal.png")
    readonly property url artUrl: coverUrl
    property url blurUrl: Qt.resolvedUrl("../../assets/cover_teal_blur.png")
    property int durationMs: 261000
    property int positionMs: 113000
    property bool isPlaying: true
    property bool shuffle: false
    property int repeatMode: 0              // 0 off, 1 context, 2 track
    // B1: like the real bridge, the observed `volume` follows a request after a delay (150 ms
    // here, `volumeLatencyMs`) and every request is echoed in order, so a stale echo can arrive
    // while the user is still dragging. The UI must not jump back to it.
    property int volume: 72
    property int lastAudibleVolume: 72
    property int volumeLatencyMs: 150
    property var volumeRequests: []          // [{v, at}] sent, not yet echoed (newest last)
    property var volumeSent: []              // PREVIEW: every value the UI requested, in order
    onVolumeChanged: if (volume > 0) lastAudibleVolume = volume
    property Timer volumeEcho: Timer {       // mock only; runs just while an echo is pending
        interval: 25
        repeat: true
        running: player.volumeRequests.length > 0
        onTriggered: {
            const now = Date.now()
            let q = player.volumeRequests
            while (q.length > 0 && now - q[0].at >= player.volumeLatencyMs) {
                player.volume = q[0].v
                q = q.slice(1)
            }
            player.volumeRequests = q
        }
    }
    property bool liked: true
    property bool canControl: true
    property bool queueEditable: true       // Q1: Velo is the active device
    property bool queueClearable: queueEditable && Account.signedIn && hasSession && trackUri.length > 0
    property string deviceName: "Spotify · ESCRITORIO-JUAN"
    property bool hasSession: true
    property string artistId: "a1"
    property string albumId: "al1"
    // B3: "loading" | "ready" | "unavailable" (the empty text belongs to "ready" with no rows).
    property string queueStatus: "ready"
    property string queueError: ""

    // E5b: DJ Livi in Velo. While Livi speaks the real backend
    // sets title/artist ("DJ Livi") and the cover to Livi's logo; the mock imitates it.
    property bool djAvailable: true
    property bool djActive: false
    property bool djSpeaking: false
    property string djLine: ""
    // PREVIEW ONLY: with djDemo on, a running DJ session alternates Livi and songs every few
    // seconds (only while playing). The real backend has no such timer.
    property bool djDemo: false
    property int djDemoStep: 0
    readonly property var djDemoLines: [
        qsTr("¡Hola, Juan! Soy Livi. Hoy arranco con algo tranquilo para que entres en calma."),
        qsTr("Volvamos a ese indie que escuchabas en marzo; te va a sonar conocido."),
        qsTr("Cambio de onda: algo con más ritmo para lo que queda de la tarde.")
    ]
    function djSpeak(index) {
        djDemoStep = index
        djLine = djDemoLines[index % djDemoLines.length]
        djSpeaking = true
        title = qsTr("DJ")
        artist = qsTr("DJ Livi")
        coverUrl = Qt.resolvedUrl("../../assets/cover_green.png")
        blurUrl = Qt.resolvedUrl("../../assets/cover_green_blur.png")
    }
    function djPlaySong() {
        djSpeaking = false
        djLine = ""
        title = djDemoStep % 2 ? "Faro sin costa" : "Marea de cobalto (versión extendida en directo)"
        artist = "Lumen Fiel"
        coverUrl = Qt.resolvedUrl("../../assets/cover_teal.png")
        blurUrl = Qt.resolvedUrl("../../assets/cover_teal_blur.png")
    }
    property Timer djDemoTimer: Timer {
        interval: 4000
        repeat: true
        running: player.djDemo && player.djActive && player.isPlaying
        onTriggered: player.djSpeaking ? player.djPlaySong() : player.djSpeak(player.djDemoStep + 1)
    }

    property string paletteBase: "#2E6F86"
    property string paletteGlow: "#7CC9E6"
    property string paletteVeilDark: "#061B24"
    property string paletteVeilLight: "#D7E8EE"
    property string paletteInkDark: "#F4FAFC"
    property string paletteInkLight: "#0D2129"
    // R5: veil alpha computed for contrast (0..1), published with paletteReady.
    property real paletteVeilDarkOpacity: 1.0
    property real paletteVeilLightOpacity: 1.0

    property ListModel queue: ListModel {
        ListElement { trackId: "q1"; uri: "spotify:track:q1"; title: "Faro sin costa"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 198000; imageUrl: ""; liked: false; artistId: "a1"; albumId: "al1" }
        ListElement { trackId: "q2"; uri: "spotify:track:q2"; title: "Insomne"; artist: "Bosque Bajo, Iria Sol"; album: "Canciones y demos 2019–2023"; durationMs: 210000; imageUrl: ""; liked: true; artistId: "a2"; albumId: "al2" }
        ListElement { trackId: "q3"; uri: "spotify:track:q3"; title: "Sal en la ventana"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 244000; imageUrl: ""; liked: false; artistId: "a1"; albumId: "al1" }
    }
    property ListModel devices: ListModel {
        ListElement { deviceId: "d1"; name: "ESCRITORIO-JUAN"; kind: "Computer"; isActive: true }
        ListElement { deviceId: "d2"; name: "Altavoz salón"; kind: "Speaker"; isActive: false }
    }
    property ListModel outputs: ListModel {
        ListElement { outputId: "output-speakers"; name: qsTr("Altavoces (Realtek Audio)"); isDefault: true }
        ListElement { outputId: "output-headphones"; name: qsTr("Auriculares (USB Audio)"); isDefault: false }
        ListElement { outputId: "output-monitor"; name: qsTr("Monitor (DisplayPort)"); isDefault: false }
    }

    signal paletteReady()
    signal error(string message)
    signal queueSaved(string playlistId, string name)

    function playPause() { if (canControl) isPlaying = !isPlaying }
    function next() { if (canControl) positionMs = 0 }
    function previous() { if (canControl) positionMs = 0 }
    function seek(ms) { if (canControl) positionMs = Math.max(0, Math.min(durationMs, ms)) }
    function setVolume(v) {
        if (!canControl) return
        const n = Math.max(0, Math.min(100, Math.round(v)))
        volumeSent = volumeSent.concat([n])
        volumeRequests = volumeRequests.concat([{v: n, at: Date.now()}])
    }
    function toggleShuffle() { if (canControl) shuffle = !shuffle }
    function cycleRepeat() { if (canControl) repeatMode = (repeatMode + 1) % 3 }
    function toggleLiked() { liked = !liked }
    function refreshQueue() {}
    function clearQueue() {
        if (!queueClearable) { error(qsTr("Solo con Velo como dispositivo.")); return }
        queue.clear()
        shuffle = false
    }
    function saveQueueAsPlaylist(name) {
        if (!Account.signedIn) { error(qsTr("Inicia sesión en Spotify para continuar.")); return }
        const title = name.trim()
        if (!title) { error(qsTr("Escribe un nombre para la playlist.")); return }
        if (!trackUri) { error(qsTr("La cola contiene elementos que no se pueden guardar en una playlist.")); return }
        const id = "mock-queue-" + Library.playlists.count
        Library.playlists.append({playlistId: id, name: title, owner: "Juan", imageUrl: "", trackCount: queue.count + 1, pinned: false})
        queueSaved(id, title)
    }
    // Q1: only with Velo as the device; `to` is the row's final index.
    function removeFromQueue(index) {
        if (queueEditable && index >= 0 && index < queue.count) queue.remove(index)
    }
    function moveInQueue(from, to) {
        if (queueEditable && from >= 0 && to >= 0 && from < queue.count && to < queue.count && from !== to)
            queue.move(from, to, 1)
    }
    function inspectQueueTrack(index) {}
    function setQueueLiked(index, saved) {
        if (index >= 0 && index < queue.count) queue.setProperty(index, "liked", saved)
    }
    function startRadio(seedUri) {}
    function refreshDevices() {}
    function refreshOutputs() {}
    function setOutput(outputId) {
        for (let i = 0; i < outputs.count; ++i)
            outputs.setProperty(i, "isDefault", outputs.get(i).outputId === outputId)
    }
    function transferTo(deviceId) {
        queueEditable = deviceId === "d1"
        for (let i = 0; i < devices.count; ++i) {
            const d = devices.get(i)
            devices.setProperty(i, "isActive", d.deviceId === deviceId)
            if (d.deviceId === deviceId)
                deviceName = "Spotify · " + d.name
        }
    }

    // R2: play the queue row `index` (the rows before it are skipped). PREVIEW: the last index asked.
    property int queueSkipIndex: -1
    function skipToQueue(index) {
        if (!canControl || index < 0 || index >= queue.count) return
        queueSkipIndex = index
        const row = queue.get(index)
        title = row.title
        artist = row.artist
        album = row.album
        trackUri = row.uri
        queue.remove(0, index + 1)
    }
}
