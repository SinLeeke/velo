pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Settings. In memory only; answers at once, never touches the network.
QtObject {
    id: settings

    property string theme: "dark"           // "system" | "light" | "dark"
    property string language: "system"      // "system" | "es" | "en"
    property bool reduceMotion: false
    property bool performanceMode: false    // S1: no motion, no cover fade, no visualizer/drift
    property bool pauseInBackground: false  // S1: effects stop when the window loses focus
    property bool performanceKeepVisualizer: false  // ronda 2 (G5): visualizer stays on in Modo rendimiento
    property bool coverBlur: true
    property bool drift: true
    property bool tintUi: true
    property bool showLyrics: true
    property bool closeToTray: true
    property bool checkUpdates: true
    property string windowControls: "mac"   // "mac" | "windows"
    property int coverCacheLimitMb: 50
    readonly property int usedCacheMb: mockUsedCacheMb
    readonly property int cacheSizeMb: usedCacheMb
    property int mockUsedCacheMb: 12

    function refreshCacheSize() {}
    function clearCache() { mockUsedCacheMb = 0 }
    function openConfigFolder() {}

    // ---- S9/N1: visualizer of the library player bar ----
    property string playerBarVisualizer: "off"   // "off" | "bars" | "wave"

    // ---- S9/N3: library order ----
    property string librarySort: "recent_played"  // recent_played | recent_added | alphabetical | creator

    // ---- S9/N2: playback on this computer ----
    property string connectDeviceName: "Velo"
    property int audioQuality: 320               // 96 | 160 | 320
    property bool normalizeVolume: true
    property bool autoplay: true
    property bool gapless: true
    property int outputBufferMs: 100             // 200 | 100 | 50
    property bool audioCacheEnabled: true
    property int audioCacheLimitMb: 1024         // 512 | 1024 | 4096
    property bool sidebarCompact: false
    property bool compactTracks: false
    property real uiZoom: 1.0                    // 0.75 … 1.5
    property string playbackStatus: "ready"      // off | connecting | ready | reconnecting | error
    // Saved vs applied: these engine settings ask for a reconnect to take effect.
    property var appliedEngine: ({ name: "Velo", quality: 320, normalize: true, autoplay: true,
                                   gapless: true, cache: true, limit: 1024 })
    readonly property bool playbackRestartRequired: connectDeviceName !== appliedEngine.name
        || audioQuality !== appliedEngine.quality || normalizeVolume !== appliedEngine.normalize
        || autoplay !== appliedEngine.autoplay || gapless !== appliedEngine.gapless
        || audioCacheEnabled !== appliedEngine.cache || audioCacheLimitMb !== appliedEngine.limit
    function reconnectPlayback() {
        appliedEngine = { name: connectDeviceName, quality: audioQuality, normalize: normalizeVolume,
                          autoplay: autoplay, gapless: gapless, cache: audioCacheEnabled, limit: audioCacheLimitMb }
        playbackStatus = "ready"
    }
    property int usedAudioCacheMb: 318
    property bool storageBusy: false
    property bool credentialsStored: true
    function clearAudioCache() { usedAudioCacheMb = 0 }
    function clearHistory() { History.clear() }

    // ---- S9/N2-proxy: draft fields confirmed together by applyProxy() ----
    property string proxyMode: "off"             // off | system | http | socks5
    property string proxyHost: ""
    property int proxyPort: 8080
    property string proxyUser: ""
    property bool proxyPasswordStored: false
    property bool proxyBusy: false
    property string proxyError: ""
    property var supportedProxyModes: ["off", "system", "http", "socks5"]
    function applyProxy(password, replacePassword) {
        if (supportedProxyModes.indexOf(proxyMode) < 0) {
            proxyError = qsTr("Ese tipo de proxy no está disponible.")
            return
        }
        if ((proxyMode === "http" || proxyMode === "socks5") && proxyHost.trim() === "") {
            proxyError = qsTr("Escribe la dirección del servidor.")
            return
        }
        proxyError = ""
        if (replacePassword) proxyPasswordStored = password !== ""
    }
    function clearProxyPassword() { proxyPasswordStored = false }

    // ---- S9/N2-eq: local DSP ----
    property bool eqEnabled: false
    property real eqPreampDb: 0
    property var eqGainsDb: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    property string eqPreset: "Flat"
    readonly property var eqPresets: ["Flat", "Classical", "Club", "Dance", "Full Bass", "Full Bass & Treble",
        "Full Treble", "Laptop Speakers/Headphones", "Large Hall", "Live", "Party", "Pop", "Reggae", "Rock",
        "Ska", "Soft", "Soft Rock", "Techno", "Bass Booster", "Bass Reducer", "Treble Booster",
        "Vocal Booster", "Small Speakers", "Spoken Word", "Loudness", "Night Listening"]
    // Sample curves (dB per band, 60 Hz … 16 kHz); presets missing here fall back to Flat.
    readonly property var mockPresetGains: ({
        "Flat": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        "Classical": [0, 0, 0, 0, 0, 0, -4.5, -4.5, -4.5, -6],
        "Club": [0, 0, 5, 3.5, 3.5, 3.5, 2, 0, 0, 0],
        "Dance": [6, 4.5, 1.5, 0, 0, -3.5, -4.5, -4.5, 0, 0],
        "Full Bass": [6, 6, 6, 3.5, 1, -2.5, -5, -6, -6.5, -6.5],
        "Pop": [-1, 3, 4.5, 5, 3.5, 0, -1.5, -1.5, -1, -1],
        "Rock": [5, 3, -3.5, -5, -2, 2.5, 5.5, 7, 7, 7],
        "Bass Booster": [5.5, 4.5, 3.5, 2, 1, 0, 0, 0, 0, 0],
        "Vocal Booster": [-1.5, -3, -3, 1.5, 3.5, 3.5, 3, 1.5, 0, -1.5],
        "Night Listening": [-2, -1, 0, 1, 2, 2, 1, 0, -1, -2]
    })
    function finite(db) { return isFinite(db) ? Math.max(-12, Math.min(12, db)) : 0 }
    function setEqBand(index, db) {
        if (index < 0 || index > 9 || !isFinite(db)) return
        const next = eqGainsDb.slice()
        next[index] = finite(db)
        eqGainsDb = next
        eqPreset = "Custom"
    }
    function setEqPreset(preset) {
        if (eqPresets.indexOf(preset) < 0) return
        eqGainsDb = (mockPresetGains[preset] || mockPresetGains["Flat"]).slice()
        eqPreampDb = 0
        eqPreset = preset
    }
    function resetEq() {
        eqGainsDb = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        eqPreampDb = 0
        eqPreset = "Flat"
    }
    onEqPreampDbChanged: if (!isFinite(eqPreampDb)) eqPreampDb = 0
}
