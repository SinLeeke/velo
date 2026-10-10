import QtQuick
import Velo
import Velo.Backend as Backend
import "nav"

// Velo main window.
// Created frameless; the native chrome (crates/app/cpp/window_chrome.cpp) restores what
// Windows 11 gives a real window: DWM shadow and rounded corners, resize edges, Snap,
// min/max animations and the system menu.
// The title bar below is a transparent strip over the content; the native hit test makes it
// drag the window except over items that take clicks.
// `Velo.Backend` is imported qualified: its `Library` and `Settings` singletons would
// otherwise clash with view names.
Window {
    id: root

    // Read by the native chrome.
    property int titleBarHeight: Theme.size.titleBar   // draggable strip height (logical px)
    property bool darkChrome: Theme.dark               // dark DWM frame / system menu
    property bool mica: false                          // set by the app when VELO_MICA=1
    property bool closeToTray: Backend.Settings.closeToTray
    property bool trayAvailable: false // actualizado por Win32; sin bandeja, cerrar termina.

    // "mac" (traffic lights, top left) | "windows" (caption buttons, top right).
    property string controlsStyle: Backend.Settings.windowControls === "windows" ? "windows" : "mac"
    // "nowplaying" | "library": the view asked for.
    property string currentView: "library"
    readonly property bool signedIn: Backend.Account.signedIn
    // "login" | "library" | "nowplaying": the view asked for, and the one the Loader shows. They
    // differ for one frame while SnapshotFade photographs the outgoing view (B1).
    readonly property string wantedView: !signedIn ? "login" : currentView
    property string shownView: ""
    property bool loginHasAppeared: false
    onWantedViewChanged: syncView()
    onSignedInChanged: if (!signedIn) {
        currentView = "library"
        pendingLinkKind = ""; pendingLinkId = ""
        queuePanel.close(); devicePicker.close(); playlistPicker.close()
        appToast.dismiss()
    }
    function syncView() {
        // S1-NP: Library ↔ Now Playing has its own choreography, which also takes a change of
        // mind halfway (wantedView back to shownView while it runs).
        if (shownView !== "" && viewMorph.takes(shownView, wantedView)) {
            viewMorph.go(wantedView === "nowplaying", () => { root.shownView = root.wantedView })
            return
        }
        if (wantedView === shownView)
            return
        if (shownView === "") {
            shownView = wantedView          // first view: no transition
            return
        }
        if (shownView === "login" && signedIn && viewLoader.item && !viewLoader.item.celebrating) {
            viewLoader.item.celebrate()
            return
        }
        if (shownView === "login" && viewLoader.item && viewLoader.item.celebrating) return
        // B1: crossfade the outgoing view's snapshot away while the incoming one settles 0.98 → 1.
        snapshotFade.run(() => { root.shownView = root.wantedView },
                         root.shownView === "login" && Theme.motion.scale === 0 ? 0 : Theme.motion.viewFade,
                         Theme.motion.easeOut, root.wantedView === "nowplaying" ? npLoader : viewLoader)
    }
    // X1b: conservar navegación externa hasta que el Loader cree la biblioteca.
    property string pendingLinkKind: ""
    property string pendingLinkId: ""
    function openPendingLink() {
        if (!signedIn || !pendingLinkKind || shownView === "login" || viewLoader.status !== Loader.Ready
                || !viewLoader.item || !("navigate" in viewLoader.item))
            return
        const kind = pendingLinkKind
        const itemId = pendingLinkId
        pendingLinkKind = ""
        pendingLinkId = ""
        showInLibrary(kind, itemId)
    }

    // Space the window controls take at each end of the title bar, for views that put their
    // own controls in that strip (e.g. the Now Playing tool pill, top right).
    readonly property real chromeLeadingInset: titleBar.leadingInset
    readonly property real chromeTrailingInset: titleBar.trailingInset

    width: Theme.size.windowWidth
    height: Theme.size.windowHeight
    // The minimum follows the interface zoom (Theme.px), capped to the screen's work area.
    minimumWidth: Math.min(Theme.size.windowMinWidth, Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : Theme.size.windowMinWidth)
    minimumHeight: Math.min(Theme.size.windowMinHeight, Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : Theme.size.windowMinHeight)
    visible: true
    title: Qt.application.name
    color: mica ? Theme.library.clearPaint : Theme.bg
    flags: Qt.Window | Qt.FramelessWindowHint

    // Settings → Appearance drives light/dark ("system" follows Windows). Every change, from any
    // view (Library and Now Playing buttons, Settings, Windows itself), goes through applyTheme:
    // A1 photographs the window first, flips the theme under the photo and fades the photo away.
    function applyTheme() {
        const mode = Backend.Settings.theme
        const systemDark = Qt.styleHints.colorScheme === Qt.ColorScheme.Dark
        const nextDark = mode === "dark" || (mode === "system" && systemDark)
        const set = () => {
            Theme.mode = Backend.Settings.theme
            Theme.systemDark = Qt.styleHints.colorScheme === Qt.ColorScheme.Dark
        }
        if (nextDark === Theme.dark) {
            set()                            // same colours ("dark" ↔ "system" while dark): nothing to fade
            return
        }
        snapshotFade.run(set, Theme.motion.themeFade, Theme.motion.easeInOut, null)
    }
    Connections {
        target: Backend.Settings
        function onThemeChanged() { root.applyTheme() }
    }
    Connections {
        target: Qt.styleHints
        function onColorSchemeChanged() { root.applyTheme() }
    }
    // Settings → "Reducir movimiento" / "Modo rendimiento": Theme.motion durations scale to 0.
    Binding {
        target: Theme
        property: "reduceMotion"
        value: Backend.Settings.reduceMotion
    }
    Binding {
        target: Theme
        property: "performanceMode"
        value: Backend.Settings.performanceMode
    }
    Binding {
        target: Theme
        property: "pauseInBackground"
        value: Backend.Settings.pauseInBackground
    }
    Binding {
        target: Theme
        property: "drift"
        value: Backend.Settings.drift
    }
    Binding { target: Theme; property: "playing"; value: Backend.Player.isPlaying }
    Binding { target: Theme; property: "windowVisible"; value: root.visible && root.visibility !== Window.Minimized }
    Binding { target: Theme; property: "windowFocused"; value: root.active }

    // S9/N2 zoom: Settings.uiZoom (Ajustes, Ctrl+Más/Menos/0) → Theme.zoom, which every length
    // token multiplies, so the whole UI lays out again at the new size (crisp, hit testing and
    // the native title-bar test intact). The old size crossfades away (SnapshotFade, as A1).
    function settingsZoom() {
        const z = Backend.Settings.uiZoom
        return isFinite(z) && z > 0 ? z : 1
    }
    function applyZoom() {
        if (Math.abs(settingsZoom() - Theme.zoom) < 0.001)
            return
        snapshotFade.run(() => { Theme.zoom = root.settingsZoom() }, Theme.motion.zoomFade, Theme.motion.easeOut, null)
    }
    function stepZoom(dir) {
        const next = Theme.nextZoom(settingsZoom(), dir)
        if (Math.abs(next - settingsZoom()) >= 0.001)
            Backend.Settings.uiZoom = next   // persisted by the bridge; applyZoom follows its signal
    }
    Connections {
        target: Backend.Settings
        ignoreUnknownSignals: true
        function onUiZoomChanged() { root.applyZoom() }
    }
    // Any view, text fields included (Ctrl+0 / Ctrl+± do nothing inside an editor anyway).
    // Ctrl+= is the unshifted "+" key of US layouts; the keypad keys match these too.
    Shortcut { sequences: [StandardKey.ZoomIn, "Ctrl+="]; context: Qt.ApplicationShortcut; onActivated: root.stepZoom(1) }
    Shortcut { sequences: [StandardKey.ZoomOut]; context: Qt.ApplicationShortcut; onActivated: root.stepZoom(-1) }
    Shortcut { sequence: "Ctrl+0"; context: Qt.ApplicationShortcut; onActivated: root.stepZoom(0) }

    // Cover palette: on every new cover and again when light/dark flips, because
    // setCoverPalette resolves the veil/ink for the mode active at call time.
    function applyPalette() {
        Theme.setCoverPalette({
            base: Backend.Player.paletteBase, glow: Backend.Player.paletteGlow,
            veilDark: Backend.Player.paletteVeilDark, veilLight: Backend.Player.paletteVeilLight,
            inkDark: Backend.Player.paletteInkDark, inkLight: Backend.Player.paletteInkLight,
            veilDarkOpacity: Backend.Player.paletteVeilDarkOpacity, veilLightOpacity: Backend.Player.paletteVeilLightOpacity,
            scrimDarkOpacity: Backend.Player.paletteScrimDarkOpacity, scrimLightOpacity: Backend.Player.paletteScrimLightOpacity
        })
    }
    Connections {
        target: Backend.Player
        function onPaletteReady() { root.applyPalette() }
    }
    Connections {
        target: Theme
        function onDarkChanged() { root.applyPalette() }
        function onPauseEffectsInBackgroundChanged() { root.reportWindowState() }
    }

    // The visualizer (and the position clock) stop while the window is minimized or hidden (nothing
    // is painted then). Losing focus keeps them running unless the user turns on "pause effects in
    // the background" (Theme.pauseInBackground, user 2026-10-08) or Modo rendimiento is on.
    function reportWindowState() {
        const shown = visibility !== Window.Minimized && visibility !== Window.Hidden
        Backend.Visualizer.setWindowState(shown, active || !Theme.pauseEffectsInBackground)
    }
    onVisibilityChanged: reportWindowState()
    onActiveChanged: reportWindowState()
    // Ronda 2 (G5): Rust paces the bands to the screen's refresh rate, which only the UI knows
    // (it changes when the window moves to another monitor): told at start and on every change.
    readonly property real displayRate: Screen.refreshRate
    function reportDisplayRate() {
        if (displayRate > 0)
            Backend.Visualizer.setDisplayRate(displayRate)
    }
    onDisplayRateChanged: reportDisplayRate()
    onScreenChanged: reportDisplayRate()
    Component.onCompleted: {
        Theme.mode = Backend.Settings.theme   // before the first frame: no theme flash
        Theme.zoom = settingsZoom()           // and no zoom jump
        shownView = wantedView
        applyPalette()
        reportWindowState()
        reportDisplayRate()
    }

    Item {
        id: content
        anchors.fill: parent

        Loader {
            id: viewLoader
            anchors.fill: parent
            asynchronous: false
            onLoaded: {
                if (root.shownView === "login") {
                    item.reveal(!root.loginHasAppeared)
                    root.loginHasAppeared = true
                }
                Qt.callLater(root.openPendingLink)
            }
            // S1-NP: the library stays alive under Now Playing (npLoader), so going back shows it
            // as it was. While Now Playing covers it, it is hidden (not painted) and disabled.
            source: root.shownView === "login" ? "views/Login.qml"
                  : root.shownView !== "" ? "views/Library.qml"
                  : ""
            visible: root.shownView !== "nowplaying" || viewMorph.busy
            // Kept enabled while the choreography runs: turning it off would flip its controls to
            // their disabled look in the first/last frame (a visible pop). morphShield takes input.
            enabled: root.shownView !== "nowplaying" || viewMorph.busy
            // B1: view changes go through syncView() → SnapshotFade (crossfade + 0.98 → 1 settle).
        }
        // S1-NP: while the choreography runs the library under it takes no input (Now Playing,
        // above, still does: its close disc can reverse the transition halfway).
        MouseArea {
            id: morphShield
            anchors.fill: parent
            enabled: viewMorph.busy
            visible: enabled
            hoverEnabled: true
            acceptedButtons: Qt.AllButtons
            onWheel: wheel => wheel.accepted = true
        }
        // S1-NP: Now Playing, built only while it is shown, over the library.
        Loader {
            id: npLoader
            anchors.fill: parent
            asynchronous: false
            active: root.shownView === "nowplaying"
            sourceComponent: NowPlaying { ownsCredits: false }   // the library's credits dialog serves both
        }
        Binding {
            target: viewLoader.item
            property: "nowPlayingVisible"
            value: root.shownView === "nowplaying"
            when: root.shownView === "library" && viewLoader.item && "nowPlayingVisible" in viewLoader.item
        }
        // S1-NP: Library ↔ Now Playing choreography (drives the two Loaders).
        NowPlayingMorph {
            id: viewMorph
            anchors.fill: parent
            library: viewLoader
            nowPlaying: npLoader
        }

        // Library → overlays and Now Playing.
        Connections {
            target: root.shownView === "login" ? viewLoader.item : null
            ignoreUnknownSignals: true
            function onSuccessFinished() {
                if (!root.signedIn) return
                snapshotFade.run(() => { root.shownView = root.wantedView },
                    Theme.motion.scale === 0 ? 0 : Theme.motion.viewFade, Theme.motion.easeOut, viewLoader)
            }
        }
        Connections {
            target: root.shownView === "library" ? viewLoader.item : null
            ignoreUnknownSignals: true
            function onNowPlayingRequested() { root.currentView = "nowplaying" }
            function onAddToPlaylistRequested(uri) { playlistPicker.openFor(uri) }
            function onQueueRequested() { queuePanel.opened ? queuePanel.close() : queuePanel.open() }
            function onDevicesRequested() { devicePicker.opened ? devicePicker.close() : devicePicker.open() }
        }

        // Now Playing → back to the library and overlays.
        Connections {
            target: npLoader.item
            ignoreUnknownSignals: true
            function onCloseRequested() { root.currentView = "library" }
            function onAddToPlaylistRequested(uri) { playlistPicker.openFor(uri) }
            // Compatibility signal: the tool pill labels this route «Volver a la biblioteca».
            function onMiniPlayerRequested() { root.currentView = "library" }
            function onQueueRequested() { queuePanel.opened ? queuePanel.close() : queuePanel.open() }
            // S9/N2: Ajustes is a library page; Now Playing folds away onto it.
            function onSettingsRequested() { root.showInLibrary("settings", "") }
        }

        // Q1: docked at the right, between the title strip and the library player bar. The view
        // narrows by `queueInset` (layout, not overlay), so nothing it shows is covered.
        QueuePanel {
            id: queuePanel
            z: 2
            // G4: it always ends above the bottom controls: the player bar in the library, the
            // visualizer band's reserve in Now Playing.
            bottomInset: root.shownView === "library" ? Theme.size.playerBar
                       : root.shownView === "nowplaying" && npLoader.item && "queueBottomReserve" in npLoader.item
                         ? npLoader.item.queueBottomReserve : 0
            // G4: its width (resizable by its left edge) always leaves the narrowest sidebar and a
            // minimum page; the library's sidebar gives way to it (Library.sidebarCeiling), never
            // the other way round, so the two widths never depend on each other in a loop.
            maxWidth: root.width - Theme.size.sidebarMin - Theme.resize.minPage
            onAddToPlaylistRequested: uri => playlistPicker.openFor(uri)
            onArtistRequested: artistId => root.showInLibrary("artist", artistId)
            onAlbumRequested: albumId => root.showInLibrary("album", albumId)
        }
        Binding {
            target: viewLoader.item
            property: "queueInset"
            value: queuePanel.opened ? queuePanel.width : 0
            when: root.shownView !== "login" && viewLoader.status === Loader.Ready
        }
        Binding {
            target: npLoader.item
            property: "queueInset"
            value: queuePanel.opened ? queuePanel.width : 0
            when: npLoader.status === Loader.Ready
        }
        // Now Playing's «Cola» button shows the docked queue as open (when the view offers it).
        Binding {
            target: npLoader.item
            property: "queueOpen"
            value: queuePanel.opened
            when: npLoader.status === Loader.Ready && "queueOpen" in npLoader.item
        }

        // Anchored above the library player bar, next to its devices button (bottom right).
        DevicePicker {
            id: devicePicker
            x: root.width - width - Theme.space.lg
            y: root.height - height - Theme.size.playerBar - Theme.space.sm
        }

        PlaylistPicker {
            id: playlistPicker
            x: (root.width - width) / 2
            y: (root.height - height) / 2
        }

        // One banner for the whole app (the login view has its own). What failed is said in
        // the backend's words; the action, when there is one, is the way out.
        Toast {
            id: appToast
            objectName: "appToast"
            property string action: ""      // what actionRequested does: "ownApp" | ""
            onActionRequested: if (action === "ownApp") root.showInLibrary("settings", "account")
            function report(text, actionKind, severity) {
                action = actionKind
                show(text, actionKind === "ownApp" ? qsTr("Usar mi propia app de Spotify") : "", severity)
            }
        }
        UpdateNotice {
            anchors.horizontalCenter: parent.horizontalCenter
            y: appToast.shown ? appToast.y + appToast.height + Theme.space.sm : Theme.size.titleBar + Theme.space.md
            z: appToast.z
        }
        Connections {
            target: Backend.Account
            function onAppWarningChanged() {
                if (Backend.Account.appWarning !== "" && root.shownView !== "login")
                    appToast.report(Backend.Account.appWarning, "ownApp", "notice")
            }
            function onAudioFailed(reason) { if (root.shownView !== "login") appToast.report(reason, "", "error") }
            function onSignInFailed(reason) { if (root.shownView !== "login") appToast.report(reason, "", "error") }
        }
        // Actions that fail (save, remove from a playlist, add to the queue, edit details,
        // playback, transfer…) arrive here; before, they failed in silence.
        Connections {
            target: Backend.Player
            function onError(message) { if (root.signedIn && message !== "") appToast.report(message, "", "error") }
        }
    }

    TitleBar {
        id: titleBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: root.titleBarHeight
        z: 4
        controlsStyle: root.controlsStyle
        ink: root.shownView === "nowplaying" ? Theme.cover.ink : Theme.text
        inkSecondary: root.shownView === "nowplaying" ? Theme.coverInkSecondary : Theme.textSecondary
        // v0.9.2: over the light cover veil the timer gets the same local scrim as the controls.
        timerScrimOpacity: root.shownView === "nowplaying" ? Theme.cover.scrimOpacity : 0
        // T8: the sleep timer stays left of the view's own buttons in the strip.
        trailingReserve: {
            const view = root.shownView === "nowplaying" ? npLoader.item : viewLoader.item
            return view && "titleTrailingReserve" in view ? view.titleTrailingReserve : 0
        }
    }
    Binding {
        target: viewLoader.item
        property: "titleTimerWidth"
        value: titleBar.timerWidth
        when: viewLoader.item !== null && "titleTimerWidth" in viewLoader.item
    }

    // A1 + B1: snapshot crossfade over the whole window, title bar included.
    SnapshotFade {
        id: snapshotFade
        anchors.fill: parent
        z: 10
        source: root.contentItem
    }

    // M1: menus anywhere ask for a page through Nav, or through Actions for the playing song.
    Connections {
        target: Nav
        function onShow(kind, itemId) { root.showInLibrary(kind, itemId) }
    }
    Connections {
        target: Backend.Actions
        function onNavigateRequested(kind, id) { root.showInLibrary(kind, id) }
    }
    Connections {
        target: Backend.Library
        function onNavigateRequested(kind, id) {
            root.pendingLinkKind = kind
            root.pendingLinkId = id
            root.currentView = "library"
            Qt.callLater(root.openPendingLink)
        }
    }
    // Pegar fuera de un editor abre el enlace; los campos conservan su Ctrl+V habitual.
    TextEdit { id: linkClipboard; visible: false }
    Shortcut {
        sequence: "Ctrl+V"
        enabled: root.signedIn && !Nav.isTextInput(root.activeFocusItem)
        onActivated: {
            linkClipboard.text = ""
            linkClipboard.paste()
            Backend.Library.openLink(linkClipboard.text)
            linkClipboard.text = ""
        }
    }
    // X1a: segunda ejecución; Qt/Win32 conservan la activación y el estado maximizado.
    Connections {
        target: Backend.Desktop
        function onShowRequested() {
            Backend.Desktop.showWindow()
        }
    }

    // "Ir al artista / álbum" from the queue: the library opens that page (from Now Playing too).
    // The library is alive under Now Playing (S1-NP): it navigates at once and Now Playing folds
    // away onto that page. While it is not built yet (login) the link waits in pendingLink and
    // opens when the library has loaded.
    function showInLibrary(kind, itemId) {
        if (shownView !== "login" && wantedView !== "login" && viewLoader.item && viewLoader.item.navigate) {
            viewLoader.item.navigate(kind, itemId, "")
            currentView = "library"
            return
        }
        pendingLinkKind = kind
        pendingLinkId = itemId
        currentView = "library"
        Qt.callLater(openPendingLink)
    }

    // Playback shortcuts (window-wide; text fields keep their own keys).
    readonly property bool canPlay: root.signedIn && Backend.Player.canControl
    // E2b: with Premium and nothing playing anywhere, play also works before Velo's audio is
    // enabled (it opens the one-time audio login, then plays).
    readonly property bool canStart: root.canPlay || (root.signedIn && Backend.Account.isPremium
                                                      && !Backend.Account.audioReady && !Backend.Player.hasSession)
    readonly property int seekStepMs: Theme.sliderInput.seekStepMs
    readonly property int volumeStep: Theme.sliderInput.volumeStep
    // Keyboard map (Ajustes should list it): Espacio reproducir/pausar · Ctrl+←/→ anterior /
    // siguiente · Shift+←/→ retroceder / avanzar · Ctrl+↑/↓ volumen · Ctrl+Shift+S aleatorio ·
    // Ctrl+Shift+R repetir · Ctrl+Shift+L me gusta · Ctrl+F / Ctrl+L buscar · Ctrl+, ajustes · Alt+←/→ y los botones
    // laterales del ratón: atrás / adelante · Ctrl+Más/Menos/0 zoom · Ctrl+V abrir un enlace.
    // Space: only while the focus is not on something that uses the key (Nav.focusUsesKeys):
    // a focused button, switch, chip, row or lyric line takes it, as Windows expects.
    Shortcut { sequence: "Space"; enabled: root.canStart && !Nav.focusUsesKeys(root.activeFocusItem); onActivated: Backend.Player.playPause() }
    Shortcut { sequence: "Ctrl+Right"; enabled: root.canPlay; onActivated: Backend.Player.next() }
    Shortcut { sequence: "Ctrl+Left"; enabled: root.canPlay; onActivated: Backend.Player.previous() }
    Shortcut { sequence: "Ctrl+Up"; enabled: root.canPlay; onActivated: Backend.Player.setVolume(Math.min(100, Backend.Player.volume + root.volumeStep)) }
    Shortcut { sequence: "Ctrl+Down"; enabled: root.canPlay; onActivated: Backend.Player.setVolume(Math.max(0, Backend.Player.volume - root.volumeStep)) }
    Shortcut { sequence: "Ctrl+Shift+S"; enabled: root.canPlay; onActivated: Backend.Player.toggleShuffle() }
    Shortcut { sequence: "Ctrl+Shift+R"; enabled: root.canPlay; onActivated: Backend.Player.cycleRepeat() }
    Shortcut { sequence: "Ctrl+Shift+L"; enabled: root.signedIn; onActivated: Backend.Player.toggleLiked() }
    Shortcut { sequence: "Shift+Right"; enabled: root.canPlay; onActivated: Backend.Player.seek(Math.min(Backend.Player.durationMs, Backend.Player.positionMs + root.seekStepMs)) }
    Shortcut { sequence: "Shift+Left"; enabled: root.canPlay; onActivated: Backend.Player.seek(Math.max(0, Backend.Player.positionMs - root.seekStepMs)) }
    // T3: Ctrl+F / Ctrl+L focus the search box of the library's top bar (from Now Playing, once
    // the library is back on screen: a hidden field cannot take the focus).
    property bool pendingSearchFocus: false
    function focusLibrarySearch() {
        if (root.shownView === "library" && viewLoader.item && "focusSearch" in viewLoader.item) {
            root.pendingSearchFocus = false
            viewLoader.item.focusSearch()
        } else {
            root.pendingSearchFocus = true
            root.currentView = "library"
        }
    }
    onShownViewChanged: if (pendingSearchFocus && shownView === "library") Qt.callLater(focusLibrarySearch)
    Shortcut { sequences: [StandardKey.Find, "Ctrl+L"]; enabled: root.signedIn; onActivated: root.focusLibrarySearch() }
    Shortcut { sequence: "Ctrl+,"; enabled: root.signedIn; onActivated: root.showInLibrary("settings", "") }
}
