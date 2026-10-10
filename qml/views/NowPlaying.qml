import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Templates as T
import Velo.Backend
import "../theme"
import "../components"
import "../components/nowplaying"
import "../components/library"
import "../components/devices"
import "../nav"
import "../components/motion/morph.js" as Morph

// Now Playing: the cover shapes the background and palette.
// Left column (~46 %): cover, title, artist, album, progress, transport, volume — centered.
// Right column: time-synced lyrics. Top: close disc (left, under the traffic lights) and the
// tool pill (right). Bottom: the visualizer band (or, with the circular style, the ring around the cover).
// S1-NP: the entrance/exit choreography reads `presence` (set by Main's NowPlayingMorph):
// 0 = folded into the library's mini player, 1 = at rest. Each element owns a window of it
// (Theme.motion.npWin*), so the exit plays the same windows backwards and reverses mid-way
// without jumps. Opacity and transforms only; at rest (presence 1) nothing is re-evaluated.
// R2 keyboard: the view takes focus when it opens; Esc goes back to the library (the queue and
// the popups close themselves first). Tab follows the screen: close, tool pill, the cover (T4), the song menu,
// progress, transport, volume, then the lyrics as one stop.
// R2 no song (no session): a short headline with a way back to the library; no lyrics column.
// T4 (user's request): a click on any free spot (not a control, the lyrics, the queue or a menu)
// rotates the visualizer: band at the bottom → ring around the cover → off → band. A click on the
// cover switches its shape (wave ↔ bars). Both only change Visualizer's persisted settings.
FocusScope {
    id: root
    objectName: "nowPlayingView"         // NowPlayingMorph drives this view (S1-NP)

    Keys.onEscapePressed: root.closeRequested()
    Component.onCompleted: { built = true; root.forceActiveFocus() }

    signal closeRequested()
    signal miniPlayerRequested()
    signal queueRequested()
    signal settingsRequested()
    signal lyricsToggled(bool shown)
    signal addToPlaylistRequested(string uri)

    property alias vizBand: band
    property alias vizRing: cover.ringSlot
    property alias songMenu: songMenu
    readonly property alias coverArt: cover

    // Q1: width taken by the docked queue panel at the right (set by Main.qml).
    property real queueInset: 0
    readonly property real contentWidth: width - queueInset
    // The queue is open exactly while it takes room (R2: the Cola tool shows it as active). When
    // it closes, the keyboard comes back here so Esc keeps working.
    readonly property bool queueOpen: queueInset > 0
    onQueueOpenChanged: if (!queueOpen && visible) root.forceActiveFocus()

    // R2: nothing playing at all: no lyrics, no controls, a way back instead.
    readonly property bool idle: !Player.hasSession
    readonly property bool effectsOn: visible && Player.isPlaying && Window.window !== null
        && Window.visibility !== Window.Minimized && Window.visibility !== Window.Hidden
        && (!Theme.pauseEffectsInBackground || Window.active)
    onEffectsOnChanged: if (!effectsOn && columnSlide.running) columnSlide.complete()
    readonly property bool lyricsShown: Settings.showLyrics && !idle
    readonly property bool circular: Visualizer.style === "circular"
    // Modo rendimiento turns the visualizer off without touching the user's switch, unless
    // Ajustes › Modo rendimiento › «Mantener el visualizador» (G5).
    readonly property bool vizOn: !idle && Visualizer.enabled && (!Theme.performanceMode || Settings.performanceKeepVisualizer)
    readonly property bool bandShown: vizOn && !circular

    // ------------------------------------------------------------ T4 clicks
    // Forced off (Modo rendimiento without «Mantener el visualizador», or nothing playing): a
    // background click changes nothing, so no hidden setting moves behind the user's back.
    readonly property bool vizForcedOff: idle || (Theme.performanceMode && !Settings.performanceKeepVisualizer)
    function rotateVisualizer() {
        if (vizForcedOff)
            return
        if (!Visualizer.enabled) {
            Visualizer.style = "normal"
            Visualizer.enabled = true
        } else if (Visualizer.style !== "circular") {
            Visualizer.style = "circular"
        } else {
            Visualizer.enabled = false
        }
    }
    function toggleVizShape() { Visualizer.shape = Visualizer.shape === "bars" ? "wave" : "bars" }
    // Controls a near miss must not count as background (also while disabled, when they let
    // the press through). Their rects grow by Theme.nowPlayingClicks.barMargin.
    function freeAt(x, y) {
        const m = Theme.nowPlayingClicks.barMargin
        // The cover button counts even while it is off (T8): a click on the art never turns into
        // a background click.
        const items = [closeButton, toolPill, seekBar, transport, volumeSlider, trackInfo.moreButton, idleAction, lyricsView, cover.artButton]
        for (const item of items) {
            if (!item.visible)
                continue
            const p = item.mapToItem(root, 0, 0)
            if (x >= p.x - m && x <= p.x + item.width + m && y >= p.y - m && y <= p.y + item.height + m)
                return false
        }
        return true
    }
    // A press that only closes an open menu or the device picker (they let it through) is not
    // a background click nor a cover click: the flag lives for that press's own event and is
    // cleared right after; the handlers read it at their press and keep it until the release
    // (T4b). The overlay guard covers any popup; aboutToHide covers this view's own two.
    property bool popupJustClosed: false
    readonly property bool pressClosesPopup: popupJustClosed || popupGuard.closing
    PopupPressGuard { id: popupGuard; overlay: root.T.Overlay.overlay }
    function notePopupClose() {
        popupJustClosed = true
        Qt.callLater(clearPopupClose)
    }
    function clearPopupClose() { popupJustClosed = false }
    // G4: space the docked queue leaves free at the bottom (Main): like the columns, it ends
    // above the visualizer band's reserve instead of covering the band, as it ends above the
    // player bar in the library.
    readonly property real queueBottomReserve: bandReserve
    // T8: the title strip's sleep timer stays left of the tool pill (Main → TitleBar).
    readonly property real titleTrailingReserve: width - toolPill.x + Theme.space.md

    // ------------------------------------------------------------ S1-NP choreography
    property real presence: 1
    // The mini player's cover in this view's coordinates (empty: no flight, the cover just fades).
    property rect morphFrom: Qt.rect(0, 0, 0, 0)
    readonly property bool morphing: presence < 1
    // The big cover has decoded (or there is none): the entrance waits for it so it never flies empty.
    readonly property bool coverReady: cover.hasArt || String(Player.coverUrl) === ""
    // Cancel ListView's pending cached delegates before the Loader destroys this context.
    // A very early reversal can close before the incubation controller finishes them.
    property bool unloading: false
    function prepareUnload() { unloading = true }
    // Eased local progress of one window of `presence` (1 at rest, without any work).
    function enterStep(win, curve) {
        return presence >= 1 ? 1 : Morph.step(presence, win, curve)
    }
    // Left column blocks rise in turn (title, progress, transport, volume).
    function riseIn(i) {
        if (presence >= 1)
            return 1
        const m = Theme.motion, start = m.npControlsStart + i * m.npControlsStep
        return Morph.step(presence, [start, start + m.npControlsSpan], m.npEase)
    }
    readonly property real topIn: enterStep(Theme.motion.npWinTop, Theme.motion.npEase)


    readonly property real bandReserve: bandShown ? Theme.viz.bandHeight / 2 : 0

    // R2 (adapt): the left column always fits between the title strip and the band. The cover
    // takes what the controls leave: its preferred size (a share of the height, coverMin..coverMax)
    // when there is room; below that the gaps tighten (`compact`) and the cover shrinks, down to
    // coverFloor at the 560 px minimum height. Heights of the controls do not depend on the cover
    // (single-line texts), so nothing here loops.
    readonly property real slotRatio: circular ? Theme.nowPlaying.ringSlotRatio : 1
    readonly property real columnRoom: leftArea.height - 2 * Theme.space.lg
    readonly property real controlsHeight: trackInfo.implicitHeight + seekBar.implicitHeight
                                           + transport.implicitHeight + volumeSlider.implicitHeight
    function gaps(tight) {
        return (circular ? Theme.space.lg : tight ? Theme.space.md : Theme.space.xxl)
             + (tight ? Theme.space.sm : Theme.space.xl) + 2 * (tight ? Theme.space.sm : Theme.space.lg)
    }
    readonly property bool compact: columnRoom < controlsHeight + gaps(false) + Theme.size.coverMin * slotRatio
    readonly property real coverSize: {
        const preferred = Math.max(Theme.size.coverMin, Math.min(Theme.size.coverMax, height * Theme.nowPlaying.coverHeightRatio))
        const fits = (columnRoom - controlsHeight - gaps(compact)) / slotRatio
        return Math.round(Math.max(Theme.nowPlaying.coverFloor, Math.min(preferred, fits)))
    }

    // The engine analyses the PCM before the volume (V1b); the drawing follows the volume the user
    // hears. R2: the gain goes to the visualizer, which applies it while drawing (no array copy
    // per frame). Bands arrive at the screen's rate, only while the visualizer runs.
    readonly property real vizGain: Math.pow(Math.max(0, Math.min(100, Player.volume)) / 100, Theme.vizStyle.volumeCurve)

    // «Teñir la interfaz». R1: Main.qml should bind it next to Theme.drift (then the library player
    // bar follows it too); this keeps Now Playing right in the meantime.
    Binding {
        target: Theme
        property: "tintUi"
        value: Settings.tintUi
        restoreMode: Binding.RestoreNone
    }
    Binding { target: Theme; property: "drift"; value: Settings.drift; restoreMode: Binding.RestoreNone }

    // S1-NP: the blurred room blooms in (opacity + scale 1.15 → 1) over the sinking library.
    CoverBackground {
        objectName: "npBackground"
        // Crossfade (easeInOut) with the library photo; the scale settles with the long ease-out.
        anchors.fill: parent
        source: Player.blurUrl
        blurOn: Settings.coverBlur
        playing: Player.isPlaying
        retainEmpty: Player.hasSession
        opacity: root.enterStep(Theme.motion.npWinBackdrop, Theme.motion.easeInOut)
        scale: Morph.mix(Theme.motion.npBloom, 1, root.enterStep(Theme.motion.npWinBackdrop, Theme.motion.npEase))
    }

    // T4: background clicks. Under everything else, so a press any control (or the lyrics list)
    // takes never reaches it; the title strip (window drag) and the docked queue are left out.
    Item {
        id: backgroundTap
        objectName: "npBackgroundTap"
        y: Theme.size.titleBar
        width: root.contentWidth
        height: root.height - y
        Accessible.role: Accessible.Pane
        Accessible.name: qsTr("Fondo de Reproduciendo ahora")
        Accessible.description: root.vizForcedOff ? "" : qsTr("Haz clic en una zona libre para cambiar el visualizador: banda, anillo alrededor de la carátula o desactivado")
        property bool armed: false
        TapHandler {
            acceptedButtons: Qt.LeftButton
            onPressedChanged: if (pressed) backgroundTap.armed = !root.pressClosesPopup
            onTapped: point => {
                const p = backgroundTap.mapToItem(root, point.position.x, point.position.y)
                if (backgroundTap.armed && root.freeAt(p.x, p.y))
                    root.rotateVisualizer()
            }
        }
    }

    // Bottom visualizer band (bars/wave), drawn under the content. The Loader means that with the
    // visualizer disabled (or the circular style) nothing is created at all. It spans the content
    // width: with the queue docked it ends at the panel instead of running under it.
    Item {
        id: band
        objectName: "vizBandSlot"
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.rightMargin: root.queueInset
        anchors.bottom: parent.bottom
        height: Theme.viz.bandHeight
        visible: root.bandShown && opacity > 0
        // S1-NP: grows from its baseline.
        readonly property real reveal: root.enterStep(Theme.motion.npWinBand, Theme.motion.npEase)
        opacity: reveal
        transform: Scale { origin.y: band.height; yScale: Morph.mix(Theme.motion.npBandFrom, 1, band.reveal) }

        Loader {
            anchors.fill: parent
            active: root.bandShown
            sourceComponent: VisualizerBand {
                bands: Visualizer.bands
                running: Visualizer.running
                gain: root.vizGain
                shape: Visualizer.shape
                color: Theme.coverVizColor
            }
        }
    }

    // Circular visualizer ring: lives in the cover's ring slot, behind the circle.
    Loader {
        parent: cover.ringSlot
        anchors.fill: parent
        active: root.circular && root.vizOn
        sourceComponent: VisualizerRing {
            bands: Visualizer.bands
            running: Visualizer.running
            gain: root.vizGain
            shape: Visualizer.shape
            color: Theme.coverVizColor
            innerRadius: cover.artSize / 2
        }
    }

    // ------------------------------------------------------------ top bar
    // Declared before the columns so Tab reaches it first (it is drawn on top through z).
    IconButton {
        id: closeButton
        objectName: "npCloseButton"
        z: 2
        x: Theme.space.lg
        y: Theme.size.titleBar - Theme.space.sm
        width: Theme.size.iconButton - Theme.space.xs
        height: width
        text: qsTr("Volver a la biblioteca")
        Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
        Keys.onReturnPressed: clicked()
        Keys.onEnterPressed: clicked()
        NowPlayingToolTip { button: closeButton }
        // With Windows caption buttons an "X" up here would read as the window's close: a chevron
        // (Now Playing folds down into the player bar). The macOS-style discs keep ref_01's X.
        iconSource: Settings.windowControls === "windows" ? Qt.resolvedUrl("../icons/expand_more.svg")
                                                         : Qt.resolvedUrl("../icons/close.svg")
        iconSize: Theme.icon.sm
        ink: Theme.cover.ink
        restFill: Theme.cover.pill
        hoverFill: Theme.cover.pillHover
        opacity: root.topIn
        transform: Translate { y: -(1 - root.topIn) * Theme.motion.npTopDrop }
        onClicked: root.closeRequested()
    }

    ToolPill {
        id: toolPill
        z: 2
        anchors.right: parent.right
        // Clear of the Windows-style caption buttons when that style is chosen.
        anchors.rightMargin: Theme.space.lg + (Settings.windowControls === "windows" ? 3 * Theme.caption.buttonWidth : 0)
        y: Theme.space.md
        opacity: root.topIn
        transform: Translate { y: -(1 - root.topIn) * Theme.motion.npTopDrop }
        lyricsActive: Settings.showLyrics
        queueActive: root.queueOpen
        devicesActive: devicePicker.opened
        dark: Theme.dark
        onDevicesClicked: devicePicker.opened ? devicePicker.close() : devicePicker.open()
        onLyricsClicked: {
            Settings.showLyrics = !Settings.showLyrics
            root.lyricsToggled(Settings.showLyrics)
        }
        onQueueClicked: root.queueRequested()
        // Persisted like the Library button; Main.qml crossfades the window (A1).
        onThemeClicked: Settings.theme = Theme.dark ? "light" : "dark"
        onSettingsClicked: root.settingsRequested()
    }

    // Spotify Connect devices, under the pill's devices button (R2).
    DevicePicker {
        id: devicePicker
        originCorner: T.Popup.TopRight
        onAboutToHide: root.notePopupClose()
        x: Math.min(root.width - width - Theme.space.lg,
                    toolPill.x + toolPill.devicesButton.x + toolPill.devicesButton.width + Theme.space.xs - width)
        y: toolPill.y + toolPill.height + Theme.space.sm
    }

    // ------------------------------------------------------------ left column
    // Lyrics toggle: the column is re-centred at once (layout) and a Translate that starts at the
    // old centre glides it to the new one (transform only). Retargets from where it is.
    // The binding settles once while the view is being built: only a real toggle slides.
    property bool built: false
    onLyricsShownChanged: {
        if (!built || !effectsOn || Theme.motion.slow <= 0)
            return
        const full = contentWidth, narrow = Math.round(contentWidth * Theme.nowPlaying.leftColumnRatio)
        columnSlide.stop()
        columnShift.x += lyricsShown ? (full - narrow) / 2 : (narrow - full) / 2
        columnSlide.from = columnShift.x
        // Making room for the lyrics it leaves quickly (easeOut), so the incoming lines never sit
        // on the cover; going back to the centre is a plain A→B move (easeInOut).
        columnSlide.easing.bezierCurve = lyricsShown ? Theme.motion.easeOut : Theme.motion.easeInOut
        columnSlide.start()
    }
    Item {
        id: leftArea
        x: 0
        y: Theme.size.titleBar
        width: root.lyricsShown ? Math.round(root.contentWidth * Theme.nowPlaying.leftColumnRatio) : root.contentWidth
        height: root.height - y - root.bandReserve
        transform: Translate { id: columnShift }
        NumberAnimation {
            id: columnSlide
            target: columnShift
            property: "x"
            to: 0
            duration: Theme.motion.slow
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.motion.easeInOut   // set per direction in onLyricsShownChanged
        }

        Column {
            id: column
            anchors.centerIn: parent
            width: Math.min(leftArea.width - 2 * Theme.space.gutter, root.coverSize + 2 * Theme.space.xxxl)
            spacing: 0

            CoverArt {
                id: cover
                objectName: "npCover"
                anchors.horizontalCenter: parent.horizontalCenter
                width: root.circular ? Math.round(root.coverSize * Theme.nowPlaying.ringSlotRatio) : root.coverSize
                height: width
                source: Player.coverUrl
                retainEmpty: Player.hasSession
                circular: root.circular
                tiltEnabled: !root.morphing
                ringReveal: root.enterStep(Theme.motion.npWinRing, Theme.motion.npEase)
                // T4: a click on the art switches the visualizer's shape (it had no click before).
                artName: Visualizer.shape === "bars" ? qsTr("Carátula: cambiar el visualizador a ola")
                                                     : qsTr("Carátula: cambiar el visualizador a barras")
                pressBlocked: root.pressClosesPopup
                // T8: only while a visualizer is drawn; otherwise the click would flip a shape
                // nobody sees (and the art is no Tab stop then).
                artEnabled: root.vizOn
                onArtClicked: root.toggleVizShape()

                // S1-NP: shared element. The cover leaves the mini player's tile (same rect, same
                // image) and travels until npCoverLand of its window: position, size and turn all
                // arrive together (x eases out faster than y, so the path arcs; it swings by
                // npCoverSpin mid-flight). Only then does it land: a short pop to npCoverPop and
                // back. Reversed on the way out (the pop becomes a little lift-off).
                readonly property real flight: root.presence >= 1 ? 1 : Morph.seg(root.presence, Theme.motion.npWinCover)
                readonly property real travel: Morph.seg(flight, [0, Theme.motion.npCoverLand])
                readonly property real landing: Morph.seg(flight, [Theme.motion.npCoverLand, 1])
                readonly property bool fromTile: root.morphFrom.width > 0 && artSize > 0
                readonly property point flightDelta: {
                    if (travel >= 1 || !fromTile)
                        return Qt.point(0, 0)
                    const c = column.mapToItem(root, x + width / 2, y + height / 2)
                    return Qt.point(root.morphFrom.x + root.morphFrom.width / 2 - c.x,
                                    root.morphFrom.y + root.morphFrom.height / 2 - c.y)
                }
                readonly property real flightScale: (fromTile
                    ? Morph.mix(root.morphFrom.width / artSize, 1, Morph.ease(Theme.motion.easeInOut, travel)) : 1)
                    * (1 + Morph.pop(landing) * (Theme.motion.npCoverPop - 1))
                opacity: fromTile && hasArt ? 1 : Morph.ease(Theme.motion.npEase, travel)
                transform: [
                    Scale {
                        origin.x: cover.width / 2
                        origin.y: cover.height / 2
                        xScale: cover.flightScale
                        yScale: cover.flightScale
                    },
                    Rotation {
                        origin.x: cover.width / 2
                        origin.y: cover.height / 2
                        angle: Math.sin(Math.PI * Morph.ease(Theme.motion.easeInOut, cover.travel)) * Theme.motion.npCoverSpin
                    },
                    Translate {
                        x: (1 - Morph.ease(Theme.motion.npEase, cover.travel)) * cover.flightDelta.x
                        y: (1 - Morph.ease(Theme.motion.easeInOut, cover.travel)) * cover.flightDelta.y
                    }
                ]
            }

            Item { width: 1; height: root.circular ? Theme.space.lg : root.compact ? Theme.space.md : Theme.space.xxl }

            // R2: nothing is playing: say so and offer the way back (the controls below hide).
            Column {
                id: idleBlock
                objectName: "npIdle"
                visible: root.idle
                width: parent.width
                spacing: Theme.space.sm
                opacity: root.riseIn(0)
                transform: Translate { y: (1 - root.riseIn(0)) * Theme.motion.npControlsRise }
                Text {
                    width: parent.width
                    text: qsTr("No suena nada")
                    color: Theme.cover.ink
                    font.family: Theme.family
                    font.pixelSize: Theme.type.heading
                    font.weight: Theme.type.heavy
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.Heading
                }
                Text {
                    width: parent.width
                    text: qsTr("Elige una canción, un álbum o una playlist en tu biblioteca.")
                    color: Theme.coverInkSecondary
                    font.family: Theme.family
                    font.pixelSize: Theme.type.body
                    lineHeight: Theme.type.bodyLineHeight
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }
                Item { width: 1; height: Theme.space.sm }
                T.AbstractButton {
                    id: idleAction
                    objectName: "npIdleAction"
                    anchors.horizontalCenter: parent.horizontalCenter
                    implicitWidth: idleLabel.implicitWidth + 2 * Theme.space.xl
                    implicitHeight: Theme.size.iconButton + Theme.space.sm
                    focusPolicy: Qt.StrongFocus
                    hoverEnabled: true
                    text: qsTr("Ir a la biblioteca")
                    Keys.onShortcutOverride: event => { if (event.key === Qt.Key_Space) event.accepted = true }
                    Keys.onReturnPressed: clicked()
                    Keys.onEnterPressed: clicked()
                    Accessible.role: Accessible.Button
                    Accessible.name: text
                    scale: down ? Theme.feel.pressScale : 1
                    Behavior on scale {
                        enabled: root.effectsOn
                        ScaleAnimator { duration: idleAction.down ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                    }
                    onClicked: root.closeRequested()
                    background: Rectangle {
                        radius: height / 2
                        color: Theme.cover.control
                        border.width: idleAction.visualFocus ? Theme.stroke.focus : 0
                        border.color: Theme.cover.ink
                    }
                    contentItem: Text {
                        id: idleLabel
                        text: idleAction.text
                        color: Theme.cover.controlInk
                        font.family: Theme.family
                        font.pixelSize: Theme.type.body
                        font.weight: Theme.type.semibold
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }

            TrackInfo {
                id: trackInfo
                visible: !root.idle
                width: parent.width
                opacity: root.riseIn(0)
                transform: Translate { y: (1 - root.riseIn(0)) * Theme.motion.npControlsRise }
                title: Player.title
                artist: Player.artist
                album: Player.album
                onMoreRequested: songMenu.popup(trackInfo, trackInfo.width / 2, trackInfo.height)
            }

            Item { visible: !root.idle; width: 1; height: root.compact ? Theme.space.sm : Theme.space.xl }

            SeekBar {
                id: seekBar
                objectName: "npSeekBar"
                visible: !root.idle
                width: parent.width
                opacity: root.riseIn(1)
                transform: Translate { y: (1 - root.riseIn(1)) * Theme.motion.npControlsRise }
                enabled: Player.canControl
                positionMs: Player.positionMs
                durationMs: Player.durationMs
                playing: Player.isPlaying
                // C5: only draw a moving wave while its existing clock runs; otherwise use a
                // deliberate straight progress line (no extra clock for remote playback).
                clock: Visualizer.bands
                clockRunning: Visualizer.running
                onSeekRequested: ms => Player.seek(ms)
            }

            Item { visible: !root.idle; width: 1; height: root.compact ? Theme.space.sm : Theme.space.lg }

            TransportControls {
                id: transport
                objectName: "npTransport"
                visible: !root.idle
                anchors.horizontalCenter: parent.horizontalCenter
                opacity: root.riseIn(2)
                transform: Translate { y: (1 - root.riseIn(2)) * Theme.motion.npControlsRise }
                playing: Player.isPlaying
                shuffle: Player.shuffle
                repeatMode: Player.repeatMode
                canControl: Player.canControl
                onPlayPauseClicked: Player.playPause()
                onPreviousClicked: Player.previous()
                onNextClicked: Player.next()
                onShuffleClicked: Player.toggleShuffle()
                onRepeatClicked: Player.cycleRepeat()
            }

            Item { visible: !root.idle; width: 1; height: root.compact ? Theme.space.sm : Theme.space.lg }

            VolumeSlider {
                id: volumeSlider
                objectName: "npVolume"
                visible: !root.idle
                anchors.horizontalCenter: parent.horizontalCenter
                opacity: root.riseIn(3)
                transform: Translate { y: (1 - root.riseIn(3)) * Theme.motion.npControlsRise }
                // T4: same width as the progress bar, its track under the progress track and
                // the speaker in the elapsed-time column: the whole block is centered.
                width: parent.width
                trackInset: seekBar.trackInset
                volume: Player.volume
                canControl: Player.canControl
                onVolumeRequested: v => Player.setVolume(v)
            }
        }
    }

    // ------------------------------------------------------------ lyrics column
    // Shown: fades in while sliding lyricsShift px from the right (base, easeOut). Hidden: the
    // reverse, shorter (fast, easeIn). Stays visible only until the fade-out ends.
    LyricsView {
        id: lyricsView
        opacity: root.lyricsShown ? 1 : 0
        visible: opacity > 0
        enabled: root.lyricsShown
        // Direction comes from the Behavior's own targetValue (set before it animates).
        Behavior on opacity {
            id: lyricsFade
            enabled: root.effectsOn
            NumberAnimation {
                duration: lyricsFade.targetValue > 0 ? Theme.motion.base : Theme.motion.fast
                easing.type: Easing.Bezier
                easing.bezierCurve: lyricsFade.targetValue > 0 ? Theme.motion.easeOut : Theme.motion.easeIn
            }
        }
        transform: Translate {
            x: root.lyricsShown ? 0 : Theme.feel.lyricsShift
            Behavior on x {
                id: lyricsSlide
                enabled: root.effectsOn
                NumberAnimation {
                    duration: lyricsSlide.targetValue === 0 ? Theme.motion.base : Theme.motion.fast
                    easing.type: Easing.Bezier
                    easing.bezierCurve: lyricsSlide.targetValue === 0 ? Theme.motion.easeOut : Theme.motion.easeIn
                }
            }
        }
        x: Math.round(root.contentWidth * Theme.nowPlaying.lyricsStartRatio)
        y: Theme.size.titleBar + Theme.space.xxxl
        width: root.contentWidth - x - Theme.space.xxxl
        height: root.height - y - Theme.space.xxxl - root.bandReserve
        model: root.unloading ? null : Lyrics.model
        // S1-NP: lines rise in one after another (LyricsView staggers them).
        intro: root.presence >= 1 ? 1 : Morph.seg(root.presence, Theme.motion.npWinLyrics)
        activeIndex: Lyrics.activeIndex
        status: Lyrics.status
        playing: Player.isPlaying
        trackKey: Player.trackUri
        plainText: Lyrics.plainText
        onSeekRequested: ms => Player.seek(ms)
    }
    // SF2: where the lyrics come from (Spotify's own, or the LRCLIB fallback).
    Text {
        visible: lyricsView.visible && Lyrics.source !== "" && (Lyrics.status === "synced" || Lyrics.status === "plain")
        opacity: lyricsView.opacity * root.enterStep(Theme.motion.npWinLyrics, Theme.motion.npEase)
        x: Math.round(root.contentWidth * Theme.nowPlaying.lyricsStartRatio) + Theme.space.lg
        y: root.height - Theme.space.xxxl - root.bandReserve + Theme.space.sm
        text: Lyrics.source === "spotify" ? qsTr("Letras de Spotify") : qsTr("Letras de LRCLIB")
        font.family: Theme.family
        font.pixelSize: Theme.type.caption
        color: Theme.coverInkLyric
    }

    // E6: "Ver créditos" (Nav.showCredits opens it). In the app the library, alive under this
    // view (S1-NP), owns the one dialog, so Main turns this off; standalone (previews) it is here.
    property bool ownsCredits: true
    Loader {
        active: root.ownsCredits
        sourceComponent: CreditsDialog {}
    }

    // M1: menu of the playing song ("…" next to the artist), like the official app.
    LibraryMenu {
        id: songMenu
        seedUri: Player.trackUri
        onAboutToHide: root.notePopupClose()
        Action {
            text: Player.liked ? qsTr("Quitar de Tus me gusta") : qsTr("Guardar en Tus me gusta")
            enabled: Player.trackUri !== ""
            onTriggered: Player.toggleLiked()
        }
        Action {
            text: qsTr("Añadir a playlist…")
            enabled: Player.trackUri !== ""
            onTriggered: root.addToPlaylistRequested(Player.trackUri)
        }
        // S9/B2: the ids of the song playing now; empty (disabled) while a new song resolves.
        Action { text: qsTr("Ir al artista"); enabled: Player.hasSession && !!Player.artistId; onTriggered: Actions.openCurrent("artist") }
        Action { text: qsTr("Ir al álbum"); enabled: Player.hasSession && !!Player.albumId; onTriggered: Actions.openCurrent("album") }
        // E6: partial credits of the playing song (CreditsDialog below).
        Action { text: qsTr("Ver créditos"); enabled: Player.trackUri.startsWith("spotify:track:"); onTriggered: Nav.showCredits(Player.trackUri) }
        LibraryMenu {
            title: Actions.sleepMinutes === 0 ? qsTr("Temporizador")
                  : Actions.sleepMinutes < 0 ? qsTr("Temporizador: al terminar la canción")
                  : qsTr("Temporizador: %n min", "", Actions.sleepMinutes)
            seedActions: false
            Action { text: qsTr("5 minutos"); onTriggered: Actions.setSleepTimer(5) }
            Action { text: qsTr("15 minutos"); onTriggered: Actions.setSleepTimer(15) }
            Action { text: qsTr("30 minutos"); onTriggered: Actions.setSleepTimer(30) }
            Action { text: qsTr("45 minutos"); onTriggered: Actions.setSleepTimer(45) }
            Action { text: qsTr("1 hora"); onTriggered: Actions.setSleepTimer(60) }
            Action { text: qsTr("Al terminar la canción"); onTriggered: Actions.setSleepTimer(-1) }
            Action { text: qsTr("Desactivar"); enabled: Actions.sleepMinutes !== 0; onTriggered: Actions.setSleepTimer(0) }
        }
    }
}
