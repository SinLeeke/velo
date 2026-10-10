pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Actions. Answers at once, never touches the network.
QtObject {
    property string inspectedUri: ""
    property int inspectedSaved: -1          // -1 unknown, 0 no, 1 yes
    property int sleepMinutes: 0             // 0 off, -1 end of song, else minutes
    property real sleepEndsAtMs: 0           // Unix ms of the deadline; 0 without a minutes timer
    signal savedChanged(string uri, bool saved)
    signal navigateRequested(string kind, string id)
    signal creditsReady(string uri, string json)
    function requestCredits(uri) {
        creditsReady(uri, JSON.stringify({title: qsTr("Canción de ejemplo"), source: "track_metadata", partial: true,
            contributors: [{name: qsTr("Artista de ejemplo"), role: "main_artist"}]}))
    }
    function startDjOn(deviceId) {}
    // E5b: DJ Livi in Velo. Starts a session (Livi opens with a line) / jumps to the next block.
    function startDj() {
        if (!Player.djAvailable) return
        Player.djActive = true
        Player.isPlaying = true
        Player.hasSession = true
        Player.djSpeak(0)
    }
    function djNextSegment() {
        if (Player.djActive) Player.djSpeak(Player.djDemoStep + 1)
    }
    property var savedSet: ({})

    function inspectSaved(uri) { inspectedUri = uri; inspectedSaved = savedSet[uri] ? 1 : 0 }
    function setSaved(uri, on) {
        const next = Object.assign({}, savedSet); next[uri] = on; savedSet = next
        // Like the real refresh of the sidebar: an unfollowed playlist leaves Library.playlists.
        if (!on && uri.startsWith("spotify:playlist:")) {
            const list = Library.playlists, id = uri.slice("spotify:playlist:".length)
            for (let i = list.count - 1; i >= 0; --i) if (list.get(i).playlistId === id) list.remove(i)
        }
        savedChanged(uri, on)
        if (uri === inspectedUri) inspectedSaved = on ? 1 : 0
    }
    function addContextToQueue(uri) {}
    // S9/B2: the playing song's artist/album, from Player.artistId/albumId (empty while the new
    // song resolves: then nothing happens, as the real bridge discards a stale answer).
    function openCurrent(kind) {
        if (kind !== "artist" && kind !== "album") return
        const known = kind === "artist" ? Player.artistId : Player.albumId
        const id = known === undefined ? (kind === "artist" ? "mock-artist" : "mock-album") : known
        if (id !== "") navigateRequested(kind, id)
    }
    function setSleepTimer(minutes) {
        sleepEndsAtMs = minutes > 0 ? Date.now() + minutes * 60000 : 0
        sleepMinutes = minutes
    }
    function shareUrl(uri) {
        const p = uri.split(":")
        return p.length === 3 && p[0] === "spotify" && p[2] !== "" ? "https://open.spotify.com/" + p[1] + "/" + p[2] : ""
    }
}
