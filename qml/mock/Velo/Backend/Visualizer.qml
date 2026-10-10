pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Visualizer. Static bands; never runs.
QtObject {
    property bool enabled: true
    property string style: "normal"         // "normal" | "circular"
    property string shape: "wave"           // "bars" | "wave"
    property real sensitivity: 1.0
    property bool running: false
    property bool simulated: false         // PCM del motor Velo; bandas estáticas en este mock
    property var bands: {
        const out = []
        for (let i = 0; i < 48; ++i)
            out.push(0.15 + 0.6 * Math.exp(-Math.pow((i - 10) / 6, 2)) + 0.45 * Math.exp(-Math.pow((i - 34) / 5, 2)))
        return out
    }

    function setWindowState(visible, focused) {}
    // Ronda 2 (G5): the UI reports the screen refresh rate; Rust paces the bands to it.
    property real displayRate: 0
    function setDisplayRate(hz) { displayRate = hz }
    // S9/N1: surfaces really on screen (Now Playing band/ring, Library player bar). The mock
    // only records them so previews can check what the UI reports.
    property bool nowPlayingVisible: false
    property bool libraryBarVisible: false
    function setSurfaces(nowPlaying, libraryBar) {
        nowPlayingVisible = nowPlaying
        libraryBarVisible = libraryBar
    }
}
