import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/library"
import "../../theme"

// Behavioral checks only: no screenshots, network or app executable.
TestCase {
    id: test
    name: "R3"
    width: 1280
    height: 720
    visible: true
    when: windowShown
    property int shortcutHits: 0
    property var savedModels: []
    property var models: [Backend.Library.quickAccess, Backend.Library.madeForYou, Backend.Library.forYou,
        Backend.Library.recent, Backend.Library.playlists, Backend.Library.topArtists,
        Backend.Library.topTracks, Backend.Library.recommendedTracks, Backend.Library.sidebarItems]
    Shortcut { sequence: "Space"; onActivated: ++test.shortcutHits }

    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }
    Component { id: sidebarComponent; LibrarySidebar { width: Theme.size.sidebar; height: test.height } }
    Component { id: homeComponent; HomePage { width: 800; height: 540 } }
    Component { id: settingsComponent; SettingsPage { width: 900; height: 600 } }
    Component { id: cardComponent; MediaCard { x: 400; y: 100; title: qsTr("Álbum de prueba"); uri: "spotify:album:example" } }
    Component { id: playComponent; LibraryPlayButton { x: 400; y: 100; text: qsTr("Reproducir") } }
    Component { id: spyComponent; SignalSpy {} }
    Component { id: volumeComponent; VolumeButton {} }
    Component { id: haloComponent; DjHalo { width: 32; height: 32 } }

    function copyRows(model) {
        const rows = []
        for (let i = 0; i < model.count; ++i) {
            const row = model.get(i), copy = {}
            for (const key in row) copy[key] = row[key]
            rows.push(copy)
        }
        return rows
    }
    function init() {
        savedModels = models.map(copyRows)
        Theme.reduceMotion = true
        Theme.zoom = 1
        Theme.mode = "dark"
        Backend.Player.canControl = true
        Backend.Player.hasSession = true
        Backend.Player.isPlaying = true
        Backend.Player.durationMs = 261000
        Backend.Player.positionMs = 60000
        Backend.Player.djAvailable = false
        Backend.Account.signedIn = true
        Backend.Account.isPremium = true
        Backend.Account.audioReady = true
        Backend.Account.appWarning = ""
        Backend.Settings.eqEnabled = true
        Backend.Library.madeForYouStatus = "ready"
        Backend.Library.sidebarStatus = "ready"
        Backend.Library.forYouAvailable = true
        shortcutHits = 0
    }
    function cleanup() {
        for (let i = 0; i < models.length; ++i) {
            models[i].clear()
            for (const row of savedModels[i]) models[i].append(row)
        }
        Theme.zoom = 1
    }
    function make(component, properties) {
        const item = createTemporaryObject(component, test, properties || {})
        verify(item !== null)
        wait(0) // one layout pass, no visual inspection
        return item
    }

    function test_seek_keyboard_and_long_episode() {
        Backend.Player.durationMs = 36000000
        const bar = make(barComponent), seek = findChild(bar, "playerBarSeek")
        seek.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Right)
        compare(Backend.Player.positionMs, 65000)
        keyClick(Qt.Key_Left)
        compare(Backend.Player.positionMs, 60000)
        keyClick(Qt.Key_PageUp)
        compare(Backend.Player.positionMs, 90000)
        keyClick(Qt.Key_PageDown)
        compare(Backend.Player.positionMs, 60000)
        keyClick(Qt.Key_Home)
        compare(Backend.Player.positionMs, 0)
        keyClick(Qt.Key_Left)
        compare(Backend.Player.positionMs, 0)
        keyClick(Qt.Key_End)
        compare(Backend.Player.positionMs, 36000000)
        keyClick(Qt.Key_Right)
        compare(Backend.Player.positionMs, 36000000)
        compare(bar.formatTime(36000000), "10:00:00")
        const duration = findChild(bar, "playerBarDuration")
        verify(duration.width >= duration.contentWidth)
        Backend.Player.canControl = false
        compare(seek.enabled, false)
    }

    function test_bar_does_not_open_empty_now_playing() {
        Backend.Player.hasSession = false
        const bar = make(barComponent), track = findChild(bar, "playerBarTrack")
        const spy = make(spyComponent, { target: bar, signalName: "nowPlayingRequested" })
        compare(track.enabled, false)
        track.clicked()
        compare(spy.count, 0)
    }

    function test_sidebar_tab_reaches_uncreated_rows() {
        const model = Backend.Library.sidebarItems
        model.clear()
        for (let i = 0; i < 40; ++i) model.append({ kind: "playlist", objectId: "p" + i,
            uri: "spotify:playlist:p" + i, name: "Playlist " + i, creator: "Juan",
            imageUrl: "", pinned: false, owned: true, addedAtMs: -1, lastPlayedAtMs: -1 })
        const sidebar = make(sidebarComponent), list = findChild(sidebar, "sidebarPlaylists")
        findChild(sidebar, "sidebarLiked").forceActiveFocus(Qt.TabFocusReason)
        for (let i = 0; i < model.count; ++i) {
            keyClick(Qt.Key_Tab)
            compare(list.currentIndex, i)
            verify(list.currentItem.activeFocus)
        }
        verify(list.contentY > 0)
        keyClick(Qt.Key_Up)
        compare(list.currentIndex, 38)
        keyClick(Qt.Key_Down)
        compare(list.currentIndex, 39)
        for (let i = 38; i >= 0; --i) {
            keyClick(Qt.Key_Backtab)
            compare(list.currentIndex, i)
        }
        keyClick(Qt.Key_Backtab)
        verify(findChild(sidebar, "sidebarLiked").activeFocus)
        const spy = make(spyComponent, { target: sidebar, signalName: "navigate" })
        keyClick(Qt.Key_Space)
        compare(shortcutHits, 0)
        compare(spy.count, 1)
        const avatar = findChild(sidebar, "sidebarAvatar")
        verify(avatar.portrait)
    }

    function test_home_loading_error_and_hidden_empty_shelves() {
        for (const model of models) model.clear()
        Backend.Library.forYouAvailable = false
        Backend.Library.madeForYouStatus = "loading"
        const home = make(homeComponent)
        verify(home.firstLoad)
        verify(findChild(home, "homeSkeleton").visible)
        for (const name of ["madeList", "mixList", "recentList", "playlistList", "artistList"])
            compare(findChild(home, "home_" + name).parent.visible, false)
        Backend.Player.error(qsTr("No se pudo conectar. Vuelve a intentar."))
        verify(findChild(home, "homeLoadError").visible)
        compare(findChild(home, "homeSkeleton").visible, false)
        Backend.Library.madeForYouStatus = "none"
        Backend.Library.sidebarStatus = "ready"
        compare(home.firstLoad, false)
    }

    function test_recent_click_plays_without_queue() {
        const home = make(homeComponent), recent = findChild(home, "homeRecent0")
        verify(recent !== null)
        const spy = make(spyComponent, { target: home, signalName: "queueRequested" })
        Backend.Player.title = qsTr("Antes del clic")
        recent.forceActiveFocus(Qt.TabFocusReason)
        mouseClick(recent)
        compare(Backend.Player.title, qsTr("Canción del enlace"))
        compare(spy.count, 0)
        recent.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Space)
        compare(shortcutHits, 0)
    }

    function test_shelf_arrows_and_vertical_wheel() {
        const model = Backend.Library.playlists
        model.clear()
        for (let i = 0; i < 12; ++i) model.append({ playlistId: "p" + i, name: "Playlist " + i,
            owner: "Juan", imageUrl: "", trackCount: 10, pinned: false, owned: true, description: "" })
        const home = make(homeComponent)
        const list = findChild(home, "home_playlistList"), viewport = findChild(home, "homeCardViewport")
        findChild(home, "homeFirstPlaylistCard").forceActiveFocus(Qt.TabFocusReason)
        mouseMove(list, list.width / 2, list.height / 2)
        const arrow = findChild(list, "shelfForward")
        tryCompare(arrow, "enabled", true)
        mouseClick(arrow)
        verify(list.contentX > 0)
        const horizontal = list.contentX, vertical = viewport.contentY
        mouseWheel(list, list.width / 2, list.height / 2, 0, -120)
        compare(list.contentX, horizontal)
        verify(viewport.contentY > vertical)
    }

    function test_settings_index_wizard_and_narrow_equalizer() {
        const settings = make(settingsComponent)
        compare(settings.wizardOpen, false)
        compare(findChild(settings, "settingsPersonalAppWizard").visible, false)
        findChild(settings, "settingsIndex_equalizer").clicked()
        compare(settings.currentSection, "equalizer")
        settings.scrollTo("account")
        findChild(settings, "settingsPersonalAppToggle").clicked()
        verify(findChild(settings, "settingsPersonalAppWizard").visible)
        findChild(settings, "settingsPersonalAppToggle").clicked()
        compare(findChild(settings, "settingsPersonalAppWizard").visible, false)
        for (const zoom of [0.75, 1, 1.5]) {
            Theme.zoom = zoom
            settings.width = Theme.resize.minPage
            wait(0)
            const index = findChild(settings, "settingsIndexChips")
            settings.scrollTo("equalizer")
            const flick = findChild(settings, "settingsViewport"), eq = findChild(settings, "settingsEqBlock")
            verify(index.visible)
            verify(index.y + index.height <= flick.y)
            verify(eq.width <= settings.width)
            if (eq.overflows) verify(eq.clip)
            findChild(settings, "settingsEqBand10").forceActiveFocus(Qt.TabFocusReason)
            const band = findChild(settings, "settingsEqBand10")
            // T8: the reveal can land a frame later in a long suite run (it failed ~1 in 2 full
            // runs with a single wait(0)); wait for it instead of sampling one turn.
            tryVerify(() => {
                const p = band.mapToItem(eq, 0, 0)
                return p.x >= 0 && p.x + band.width <= eq.width + 1
            }, 1000, "zoom=" + zoom + " x=" + band.mapToItem(eq, 0, 0).x + " band=" + band.width + " viewport=" + eq.width)
        }
        const preset = findChild(settings, "settingsPresetRock")
        preset.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Space)
        compare(shortcutHits, 0)
        compare(Backend.Settings.eqPreset, "Rock")
    }

    function test_media_play_and_space_override() {
        const card = make(cardComponent)
        mouseMove(card)
        tryCompare(card, "playShown", true)
        const play = findChild(card, "mediaCardPlay")
        mouseClick(play)
        compare(Backend.Library.lastPlayedContext, "spotify:album:example")
        const disc = make(playComponent)
        disc.forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Space)
        compare(shortcutHits, 0)
    }

    function test_volume_timer_and_dj_stop_in_pause() {
        const volume = make(volumeComponent), settle = findChild(volume, "volumeSettle")
        Backend.Player.volumeLatencyMs = 200
        volume.setVolume(43)
        verify(settle.running)
        Backend.Player.isPlaying = false
        compare(settle.running, false)
        volume.setVolume(44)
        compare(settle.running, false)
        Backend.Player.volumeLatencyMs = 0
        Backend.Player.djActive = true
        Backend.Player.djSpeaking = true
        const halo = make(haloComponent)
        compare(halo.breathing, false)
        Backend.Player.djActive = false
        Backend.Player.djSpeaking = false
    }
}
