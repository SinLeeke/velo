import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"
import ".."
import "../nowplaying"
import "../motion"

Rectangle {
    id: bar
    // S1-NP: NowPlayingMorph finds the bar by name and flies the big cover out of `coverTile`.
    objectName: "libraryPlayerBar"
    readonly property Item coverTile: tileSlot
    signal nowPlayingRequested()
    signal queueRequested()
    signal devicesRequested()
    // T5: the «Reproduciendo» side panel button (state kept by the library).
    property bool sidePanelActive: false
    signal sidePanelToggled()
    Keys.onShortcutOverride: event => {
        const w = bar.Window.window
        if (event.key === Qt.Key_Space && w && w.activeFocusItem && w.activeFocusItem.activeFocusOnTab)
            event.accepted = true
    }
    color: Theme.surface1
    implicitHeight: Theme.size.playerBar
    Rectangle { width: parent.width; height: Theme.stroke.hairline; color: Theme.hairline }

    // ---- S9/N1: visualizer behind the controls (Settings.playerBarVisualizer: off/bars/wave).
    // R3: chosen from the bar's context menu (right click anywhere on the bar) or in Ajustes.
    // T4 (user's request): a plain click on free space cycles it again (off → bars → wave → off),
    // but only clear of every control by Theme.nowPlayingClicks.barMargin, so a click that just
    // misses a button (the reason R3 had removed it) does nothing. The free-space TapHandler sits
    // under the controls: a press a control takes never reaches it.
    // The band only exists while it can be seen (Loader) and only repaints when
    // Visualizer.bands changes (at the screen's rate, only while Visualizer.running;
    // VisualizerBand glides between slower updates), so with the music paused or the window
    // minimized nothing runs here. Modo rendimiento removes it unless
    // Ajustes › Modo rendimiento › «Mantener el visualizador» is on.
    readonly property string vizMode: Backend.Settings.playerBarVisualizer
    function setVisualizer(mode) { Backend.Settings.playerBarVisualizer = mode }
    function cycleVisualizer() {
        setVisualizer(vizMode === "off" ? "bars" : vizMode === "bars" ? "wave" : "off")
    }
    // Contract S9/N1: Visualizer.setSurfaces(nowPlayingVisible, libraryBarVisible). The bar
    // reports its own visibility; Main binds `nowPlayingVisible` (through Library) to the real Now
    // Playing state, and this bar is the only caller of setSurfaces.
    property bool nowPlayingVisible: !bar.visible
    function reportSurfaces() { Backend.Visualizer.setSurfaces(nowPlayingVisible, bar.visible) }
    onVisibleChanged: Qt.callLater(reportSurfaces)
    onNowPlayingVisibleChanged: Qt.callLater(reportSurfaces)
    // Modo rendimiento keeps the visualizer only with «Mantener el visualizador» (G5).
    readonly property bool vizAllowed: !Theme.performanceMode || Backend.Settings.performanceKeepVisualizer
    readonly property real vizOpacity: Theme.playerViz.opacity
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: point => vizMenu.popup(bar, point.position.x, point.position.y)
    }
    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Barra de reproducción")
    Accessible.description: vizAllowed ? qsTr("Haz clic en una zona libre de la barra para cambiar el visualizador: espectro, forma de onda o desactivado") : ""
    // T4: free space = not within barMargin of the song button, the transport buttons, the
    // progress slider or the action buttons (their rows include the gaps between buttons).
    function freeAt(x, y) {
        const m = Theme.nowPlayingClicks.barMargin
        for (const item of [track, transportButtons, seek, actions]) {
            const p = item.mapToItem(bar, 0, 0)
            if (x >= p.x - m && x <= p.x + item.width + m && y >= p.y - m && y <= p.y + item.height + m)
                return false
        }
        return true
    }
    // T4b: the click that closes the bar's menu or a panel (output, volume…) is not a free click.
    // T8: like Now Playing's background, nothing changes while Modo rendimiento hides the band
    // (a setting nobody can see would move behind the user's back).
    PopupPressGuard { id: popupGuard; overlay: bar.T.Overlay.overlay }
    TapHandler {
        objectName: "playerBarFreeTap"
        acceptedButtons: Qt.LeftButton
        enabled: bar.vizAllowed
        property bool armed: false
        onPressedChanged: if (pressed) armed = !popupGuard.closing
        onTapped: point => { if (armed && bar.freeAt(point.position.x, point.position.y)) bar.cycleVisualizer() }
    }
    readonly property alias vizMenu: vizMenu
    Menu {
        id: vizMenu
        objectName: "playerBarVizMenu"
        // Parented to its button by popup(): pressing the button again toggles it shut.
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
        transformOrigin: Popup.Bottom
        enter: null
        exit: null
        property PopupEntrance entrance: PopupEntrance { popup: vizMenu; fromScale: Theme.entrance.menuScale; duration: Theme.motion.instant }
        implicitWidth: Theme.trackList.menuWidth
        padding: Theme.space.xs
        font.family: Theme.family
        font.pixelSize: Theme.type.body
        background: SortMenu.PopoverBackground {}
        SortMenu.SectionLabel { text: qsTr("Visualizador de la barra") }
        SortMenu.Choice { objectName: "playerBarVizOff"; text: qsTr("Desactivado"); selected: bar.vizMode === "off"; onTriggered: bar.setVisualizer("off") }
        SortMenu.Choice { objectName: "playerBarVizBars"; text: qsTr("Espectro"); selected: bar.vizMode === "bars"; onTriggered: bar.setVisualizer("bars") }
        SortMenu.Choice { objectName: "playerBarVizWave"; text: qsTr("Forma de onda"); selected: bar.vizMode === "wave"; onTriggered: bar.setVisualizer("wave") }
    }
    Loader {
        id: vizLoader
        objectName: "playerBarViz"
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: parent.height - Theme.stroke.hairline
        // G4: the drawing stays inside the bar (VisualizerBand bends its crests under the top
        // edge); the clip is a cheap scissor that also guarantees it for any overshoot.
        clip: true
        active: bar.visible && bar.vizMode !== "off" && bar.vizAllowed
        sourceComponent: VisualizerBand {
            id: band
            readonly property real gap: Theme.space.xs
            shape: bar.vizMode === "wave" ? "wave" : "bars"
            bands: Backend.Visualizer.bands
            color: Backend.Player.hasSession ? Theme.coverVizColor : Theme.accent
            barCount: Math.max(0, Math.floor(width / (barWidth + gap)))
            outline: true
            // S1: appears when cycled (or when its sound starts): fades in to its low resting
            // opacity while it grows from its baseline (Scale, base, easeOut); a pause fades it
            // out (fast) and leaves it still. Only opacity/transform, one-shot animations.
            property bool shown: false
            opacity: shown && Backend.Visualizer.running ? bar.vizOpacity : 0
            Behavior on opacity {
                OpacityAnimator { duration: Math.max(1, band.opacity > 0 ? Theme.motion.fast : Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
            }
            transform: Scale { id: grow; origin.y: band.height; yScale: 1 }
            NumberAnimation {
                id: growIn
                target: grow; property: "yScale"; from: Theme.motion.npBandFrom; to: 1
                duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut
            }
            function enter() { if (Theme.motion.base > 0) growIn.restart() }
            onShapeChanged: enter()
            Component.onCompleted: { shown = true; enter() }
        }
    }

    // ---- C8: crossfade on song change (see the track button below).
    // Only with motion on and the window on screen; otherwise the change is instant.
    function formatTime(ms) {
        const total = Math.max(0, Math.floor(ms / 1000))
        const h = Math.floor(total / 3600), m = Math.floor(total / 60) % 60, sec = ("0" + total % 60).slice(-2)
        return h > 0 ? qsTr("%1:%2:%3").arg(h).arg(("0" + m).slice(-2)).arg(sec) : qsTr("%1:%2").arg(m).arg(sec)
    }
    function animatable() {
        const w = bar.Window.window
        return Theme.motion.base > 0 && bar.visible && w !== null && w.visible && w.visibility !== Window.Minimized
    }
    property string heldTitle: ""
    property string heldArtist: ""
    property color heldArtistColor: Theme.textSecondary
    function crossText() {
        if (liveTitle.text === heldTitle && liveArtist.text === heldArtist)
            return
        ghostTitle.text = heldTitle
        ghostArtist.text = heldArtist
        ghostArtist.color = heldArtistColor
        heldTitle = liveTitle.text
        heldArtist = liveArtist.text
        heldArtistColor = liveArtist.color
        textFade.stop()
        if (!animatable() || ghostTitle.text === "") {
            ghostText.opacity = 0
            liveText.opacity = 1
            return
        }
        ghostText.opacity = 1
        liveText.opacity = 0
        textFade.start()
    }
    ParallelAnimation {
        id: textFade
        OpacityAnimator { target: ghostText; to: 0; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        OpacityAnimator { target: liveText; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
    }

    property bool coverOnA: true          // which tile shows the current cover
    property bool coverPending: false     // the other tile is loading the next one
    readonly property Item coverFront: coverOnA ? artA : artB
    readonly property Item coverBack: coverOnA ? artB : artA
    function nextCover() {
        coverFade.stop()
        coverFront.opacity = 1
        coverBack.opacity = 0
        coverBack.source = Backend.Player.coverUrl
        coverPending = true
        Qt.callLater(showCover)
    }
    function showCover() {
        if (!coverPending || coverBack.status === Image.Loading)
            return
        coverPending = false
        const incoming = coverBack, outgoing = coverFront
        coverOnA = !coverOnA
        if (!animatable()) {
            incoming.opacity = 1
            outgoing.opacity = 0
            return
        }
        coverIn.target = incoming
        coverOut.target = outgoing
        coverFade.start()
    }
    Connections {
        target: Backend.Player
        function onCoverUrlChanged() { bar.nextCover() }
    }
    Connections {
        target: bar.coverBack
        function onStatusChanged() { bar.showCover() }
    }
    ParallelAnimation {
        id: coverFade
        OpacityAnimator { id: coverIn; to: 1; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
        OpacityAnimator { id: coverOut; to: 0; duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeInOut }
    }
    Component.onCompleted: {
        reportSurfaces()
        artA.source = Backend.Player.coverUrl
        heldTitle = liveTitle.text
        heldArtist = liveArtist.text
        heldArtistColor = liveArtist.color
    }
    LibraryButton {
        id: track
        objectName: "playerBarTrack"
        anchors.left: parent.left
        anchors.leftMargin: Theme.space.lg
        anchors.verticalCenter: parent.verticalCenter
        width: Math.min(bar.width / 3, Theme.size.coverMax + Theme.space.xxl)
        height: Theme.size.trackRow
        // E2: with nothing playing and Velo's own audio off, the button offers to turn it on.
        readonly property bool offerAudio: !Backend.Player.hasSession && Backend.Account.signedIn && !Backend.Account.audioReady
        text: offerAudio ? qsTr("Activar la reproducción en Velo") : Backend.Player.hasSession
            ? qsTr("Reproduciendo ahora: %1").arg(Backend.Player.title) : qsTr("Elige algo de tu biblioteca")
        enabled: Backend.Player.hasSession || offerAudio
        leftPadding: 0
        rightPadding: 0
        // C8: song change crossfades (opacity, base, easeInOut), never slides the text.
        // Art: two tiles take turns; the new cover loads in the hidden one and fades in once it
        // is decoded, so the slot never flashes the placeholder. Text: a "ghost" copy of the old
        // lines fades out over the live ones. One shot; a new change retargets.
        contentItem: Row {
            spacing: Theme.space.md
            Item {
                id: tileSlot
                width: Theme.size.trackRow
                height: width
                LibraryArtwork { id: artA; anchors.fill: parent }
                LibraryArtwork { id: artB; anchors.fill: parent; opacity: 0 }
            }
            Item {
                anchors.verticalCenter: parent.verticalCenter
                width: track.width - Theme.size.trackRow - Theme.space.md
                height: liveText.implicitHeight
                Column {
                    id: ghostText
                    width: parent.width
                    spacing: Theme.space.xs
                    opacity: 0
                    LibraryLabel { id: ghostTitle; width: parent.width; font.weight: Theme.type.semibold }
                    LibraryLabel { id: ghostArtist; width: parent.width; font.pixelSize: Theme.type.caption }
                }
                Column {
                    id: liveText
                    width: parent.width
                    spacing: Theme.space.xs
                    LibraryLabel { id: liveTitle; width: parent.width; text: Backend.Player.hasSession ? Backend.Player.title : track.offerAudio ? qsTr("Activar la reproducción en Velo") : qsTr("Todo listo para escuchar"); font.weight: Theme.type.semibold
                        onTextChanged: Qt.callLater(bar.crossText) }
                    LibraryLabel { id: liveArtist; width: parent.width; text: Backend.Player.hasSession ? Backend.Player.artist
                        : track.offerAudio ? (Backend.Account.audioBusy ? qsTr("Termina el inicio de sesión en el navegador") : qsTr("Escucha aquí sin abrir la app de Spotify"))
                        : Backend.Account.audioReady ? qsTr("Elige algo de tu biblioteca") : qsTr("Inicia sesión para escuchar"); color: track.offerAudio ? Theme.accent : Theme.textSecondary; font.pixelSize: Theme.type.caption
                        onTextChanged: Qt.callLater(bar.crossText) }
                }
            }
        }
        onClicked: {
            if (track.offerAudio) Backend.Account.enableAudio()
            else if (Backend.Player.hasSession) bar.nowPlayingRequested()
        }
    }
    Column {
        id: transport
        anchors.left: track.right
        anchors.right: actions.left
        anchors.leftMargin: Theme.space.xl
        anchors.rightMargin: Theme.space.xl
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.space.xs
        Row {
            id: transportButtons
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Theme.space.lg
            // v0.9.2: shuffle and repeat flank previous/next, as in Now Playing (accent when on).
            // Narrow windows keep only previous/play/next, so the row never runs under the title.
            readonly property real fullWidth: 4 * Math.max(Theme.size.iconButton, Theme.size.minimumHitTarget)
                                              + playButton.width + 4 * spacing
            readonly property bool roomy: transport.width >= fullWidth
            IconButton {
                objectName: "playerBarShuffle"
                visible: transportButtons.roomy
                text: qsTr("Aleatorio")
                iconSource: Qt.resolvedUrl("../../icons/shuffle.svg")
                toggle: true
                active: Backend.Player.shuffle
                activeInk: Theme.accent
                enabled: Backend.Player.canControl
                onClicked: Backend.Player.toggleShuffle()
            }
            IconButton { text: qsTr("Anterior"); iconSource: Qt.resolvedUrl("../../icons/skip_previous.svg"); enabled: Backend.Player.canControl; onClicked: Backend.Player.previous() }
            LibraryPlayButton {
                id: playButton
                objectName: "playerBarPlay"
                text: Backend.Player.isPlaying ? qsTr("Pausar") : qsTr("Reproducir")
                iconSource: Backend.Player.isPlaying ? Qt.resolvedUrl("../../icons/pause_fill.svg") : Qt.resolvedUrl("../../icons/play_arrow_fill.svg")
                // E2b: also before Velo's audio is enabled (play opens its one-time login).
                enabled: Backend.Player.canControl || (Backend.Account.signedIn && Backend.Account.isPremium
                         && !Backend.Account.audioReady && !Backend.Player.hasSession)
                onClicked: Backend.Player.playPause()
            }
            IconButton { text: qsTr("Siguiente"); iconSource: Qt.resolvedUrl("../../icons/skip_next.svg"); enabled: Backend.Player.canControl; onClicked: Backend.Player.next() }
            IconButton {
                objectName: "playerBarRepeat"
                visible: transportButtons.roomy
                text: Backend.Player.repeatMode === 0 ? qsTr("Repetir")
                    : Backend.Player.repeatMode === 1 ? qsTr("Repetir canción") : qsTr("Desactivar repetición")
                iconSource: Backend.Player.repeatMode === 2 ? Qt.resolvedUrl("../../icons/repeat_one.svg") : Qt.resolvedUrl("../../icons/repeat.svg")
                toggle: true
                active: Backend.Player.repeatMode > 0
                activeInk: Theme.accent
                enabled: Backend.Player.canControl
                Accessible.description: Backend.Player.repeatMode === 0 ? qsTr("Repetición desactivada")
                                      : Backend.Player.repeatMode === 1 ? qsTr("Repitiendo la lista") : qsTr("Repitiendo la canción")
                // Shape changes with the mode (repeat -> repeat_one); each step up pops, switching off does not.
                property int lastMode: Backend.Player.repeatMode
                readonly property int mode: Backend.Player.repeatMode
                onModeChanged: { if (mode > 0 && mode !== lastMode && visible) pop(); lastMode = mode }
                onClicked: Backend.Player.cycleRepeat()
            }
        }
        // m:ss, or h:mm:ss for episodes of an hour or more (R3). Both times share the width of
        // the longest one, measured once per length class (TextMetrics, no per-frame work).
        TextMetrics {
            id: timeMetrics
            font.family: Theme.family
            font.pixelSize: Theme.type.micro
            font.features: Theme.tabularFeatures
            text: bar.formatTime(Backend.Player.durationMs).replace(/[0-9]/g, "0")
        }
        Row {
            width: parent.width
            spacing: Theme.space.sm
            readonly property real timeWidth: Math.ceil(timeMetrics.advanceWidth)
            LibraryLabel {
                objectName: "playerBarPosition"
                anchors.verticalCenter: parent.verticalCenter
                width: parent.timeWidth
                horizontalAlignment: Text.AlignRight
                text: bar.formatTime(seek.pressed ? seek.value : Backend.Player.positionMs)
                font.pixelSize: Theme.type.micro
                font.features: Theme.tabularFeatures
                color: Theme.textSecondary
                Accessible.ignored: true
            }
            Slider {
                id: seek
                objectName: "playerBarSeek"
                width: parent.width - 2 * parent.timeWidth - 2 * parent.spacing
                height: Theme.space.lg
                from: 0
                to: Math.max(1, Backend.Player.durationMs)
                // R3: arrows move 5 s (Theme.sliderInput, like Reproduciendo ahora's SeekBar);
                // RePág/AvPág 30 s; Inicio/Fin jump to the ends.
                stepSize: Theme.sliderInput.seekStepMs
                focusPolicy: Qt.StrongFocus
                // While dragging, the handle follows the pointer, not the player position.
                Binding { target: seek; property: "value"; value: Backend.Player.positionMs; when: !seek.pressed }
                // DJ Livi's voice is not a track: the bar stays locked while she speaks.
                enabled: Backend.Player.canControl && Backend.Player.durationMs > 0 && !Backend.Player.djSpeaking
                // A-B1: the dimming follows `enabled` (which includes DJ Livi speaking) with a short
                // fade (opacity, `fast`, easeOut) instead of jumping.
                opacity: enabled ? 1 : Theme.library.disabledOpacity
                Behavior on opacity {
                    OpacityAnimator { duration: Math.max(1, Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                }
                Accessible.name: qsTr("Progreso de la canción")
                Accessible.description: qsTr("%1 de %2").arg(bar.formatTime(value)).arg(bar.formatTime(Backend.Player.durationMs))
                // Pointer gestures seek on release; keyboard steps accumulate on the visible
                // value even when the backend has not acknowledged the previous request yet.
                onPressedChanged: if (!pressed) Backend.Player.seek(Math.round(value))
                function seekTo(ms) {
                    value = Math.max(0, Math.min(Backend.Player.durationMs, Math.round(ms)))
                    Backend.Player.seek(Math.round(value))
                }
                // Window shortcuts (Space, Shift+arrows) never steal the keys the slider uses.
                Keys.onShortcutOverride: event => {
                    if (event.modifiers === Qt.NoModifier && [Qt.Key_Left, Qt.Key_Right, Qt.Key_Up, Qt.Key_Down,
                            Qt.Key_PageUp, Qt.Key_PageDown, Qt.Key_Home, Qt.Key_End].indexOf(event.key) >= 0)
                        event.accepted = true
                }
                Keys.onPressed: event => {
                    if (!enabled || event.modifiers !== Qt.NoModifier) return
                    switch (event.key) {
                    case Qt.Key_Right:
                    case Qt.Key_Up: seekTo(value + Theme.sliderInput.seekStepMs); break
                    case Qt.Key_Left:
                    case Qt.Key_Down: seekTo(value - Theme.sliderInput.seekStepMs); break
                    case Qt.Key_PageUp: seekTo(value + Theme.sliderInput.seekPageMs); break
                    case Qt.Key_PageDown: seekTo(value - Theme.sliderInput.seekPageMs); break
                    case Qt.Key_Home: seekTo(0); break
                    case Qt.Key_End: seekTo(Backend.Player.durationMs); break
                    default: return
                    }
                    event.accepted = true
                }
                Accessible.onIncreaseAction: seekTo(value + Theme.sliderInput.seekStepMs)
                Accessible.onDecreaseAction: seekTo(value - Theme.sliderInput.seekStepMs)
                background: Rectangle {
                    x: seek.leftPadding
                    y: (seek.height - height) / 2
                    width: seek.availableWidth
                    height: Theme.stroke.progress
                    radius: height / 2
                    color: Theme.library.controlTrack
                    Rectangle { width: parent.width * seek.visualPosition; height: parent.height; radius: height / 2; color: Theme.accent }
                }
                handle: Rectangle {
                    id: handle
                    x: seek.leftPadding + seek.visualPosition * (seek.availableWidth - width)
                    y: (seek.height - height) / 2
                    width: Theme.space.sm
                    height: width
                    radius: width / 2
                    color: Theme.accent
                    // A-B2: the knob fades and grows in from `dotFrom` (instant in, fast out, easeOut)
                    // with the mouse; keyboard focus shows it at once (keys are never animated).
                    readonly property bool shown: seek.hovered || seek.pressed || seek.visualFocus
                    readonly property bool byKeys: seek.visualFocus && !seek.hovered && !seek.pressed
                    visible: shown || opacity > 0
                    opacity: shown ? 1 : 0
                    scale: shown ? 1 : Theme.feel.dotFrom
                    Behavior on opacity {
                        OpacityAnimator { duration: Math.max(1, handle.byKeys ? 0 : (handle.shown ? Theme.motion.instant : Theme.motion.fast)); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                    Behavior on scale {
                        ScaleAnimator { duration: Math.max(1, handle.byKeys ? 0 : (handle.shown ? Theme.motion.instant : Theme.motion.fast)); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                    // Keyboard focus: a ring around the knob, a gap of background between them.
                    Rectangle {
                        anchors.centerIn: parent
                        width: parent.width + 4 * Theme.stroke.focus
                        height: width
                        radius: width / 2
                        visible: seek.visualFocus
                        color: Theme.library.clearPaint
                        border.width: Theme.stroke.focus
                        border.color: Theme.focusRing
                    }
                }
            }
            LibraryLabel {
                objectName: "playerBarDuration"
                anchors.verticalCenter: parent.verticalCenter
                width: parent.timeWidth
                text: bar.formatTime(Backend.Player.durationMs)
                font.pixelSize: Theme.type.micro
                font.features: Theme.tabularFeatures
                color: Theme.textSecondary
                Accessible.ignored: true
            }
        }
    }
    Row {
        id: actions
        anchors.right: parent.right
        anchors.rightMargin: Theme.space.lg
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.space.sm
        IconButton { id: likeButton; text: Backend.Player.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Añadir a Tus me gusta"); iconSource: Backend.Player.liked ? Qt.resolvedUrl("../../icons/favorite_fill.svg") : Qt.resolvedUrl("../../icons/favorite.svg"); ink: Backend.Player.liked ? Theme.accent : Theme.text; enabled: Backend.Player.hasSession
            // C6: saving pops the heart (1 → likePop → 1); removing does not.
            onClicked: { if (!Backend.Player.liked) pop(); else unpop(); Backend.Player.toggleLiked() } }
        // A-S3: a like that arrives from elsewhere (menu, search, another surface) pops the heart too.
        Connections {
            target: Backend.Actions
            function onSavedChanged(uri, saved) { if (saved && uri === Backend.Player.trackUri) likeButton.pop() }
        }
        VolumeButton { objectName: "playerBarVolume" }
        AudioOutputButton { objectName: "playerBarOutput" }
        IconButton {
            id: visualizerButton
            objectName: "playerBarVisualizerButton"
            text: qsTr("Visualizador de la barra")
            iconSource: Qt.resolvedUrl("../../icons/graphic_eq.svg")
            onClicked: vizMenu.opened ? vizMenu.close()
                                      : vizMenu.popup(visualizerButton, width - vizMenu.width, -vizMenu.height - Theme.space.sm)
        }
        IconButton {
            objectName: "playerBarSidePanel"
            text: bar.sidePanelActive ? qsTr("Ocultar el panel Reproduciendo") : qsTr("Mostrar el panel Reproduciendo")
            iconSource: Qt.resolvedUrl("../../icons/right_panel_open.svg")
            active: bar.sidePanelActive
            toggle: true
            activeInk: Theme.accent
            onClicked: bar.sidePanelToggled()
        }
        // B-2: lyrics toggle (as in Spotify's bar). It flips Settings.showLyrics (the Now Playing
        // lyrics column) and, when turning them on with the right panel closed, opens that panel,
        // whose lyrics card then shows them. Active = lyrics on (shape + colour confirm the state).
        IconButton {
            id: lyricsButton
            objectName: "playerBarLyrics"
            text: qsTr("Letra")   // stable name: as a toggle the screen reader already announces on/off
            iconSource: Qt.resolvedUrl("../../icons/lyrics.svg")
            active: Backend.Settings.showLyrics
            toggle: true
            activeInk: Theme.accent
            enabled: Backend.Player.hasSession
            onClicked: {
                Backend.Settings.showLyrics = !Backend.Settings.showLyrics
                if (Backend.Settings.showLyrics && !bar.sidePanelActive)
                    bar.sidePanelToggled()
            }
        }
        // Mixear (crossfade) for what is playing: playlists and Tus me gusta only, and only when Velo's own
        // engine can mix (gapless on, no DJ).
        IconButton {
            objectName: "playerBarMix"
            visible: Backend.Player.mixAvailable && /^spotify:(playlist:|collection|user:[^:]+:collection)/.test(Backend.Player.contextUri)
            text: qsTr("Mixear")   // stable name: as a toggle the screen reader already announces on/off
            iconSource: Qt.resolvedUrl("../../icons/tune.svg")
            toggle: true
            active: Backend.Library.mixPlaying
            activeInk: Theme.accent
            onClicked: Backend.Library.toggleMixPlaying()
        }
        IconButton { text: qsTr("Cola de reproducción"); iconSource: Qt.resolvedUrl("../../icons/queue_music.svg"); onClicked: bar.queueRequested() }
        // The device picker closes on any outside press: if this very press closed it, the click
        // must not reopen it (PopupPressGuard, as for the free taps).
        IconButton {
            objectName: "playerBarDevices"
            text: qsTr("Conectar a un dispositivo"); iconSource: Qt.resolvedUrl("../../icons/devices.svg")
            property bool armed: true
            onPressedChanged: if (pressed) armed = !popupGuard.closing
            onClicked: if (armed) bar.devicesRequested()
        }
        // E5b: DJ Livi (start / «Elige tú, Livi»); absent without DJ on the account.
        DjButton {}
    }
}
