import QtQuick
import QtTest
import Velo.Backend
import "../../views" as Views
import "../../components/nowplaying"
import "../../components/library"
import "../../theme"
import "../../components/nowplaying/vizshape.js" as Viz

// v0.9.2: geometry and UI state with the static backend mock; no network or Rust.
TestCase {
    id: test
    name: "V92"
    visible: true
    width: 1100
    height: 760
    when: windowShown

    Component { id: backgroundComponent; CoverBackground { width: 1000; height: 700; playing: false } }
    Component { id: npComponent; Views.NowPlaying { width: test.width; height: 600; ownsCredits: false } }
    Component { id: barComponent; LibraryPlayerBar { width: test.width; height: Theme.size.playerBar } }

    property var savedState
    function initTestCase() {
        savedState = {
            mode: Theme.mode, zoom: Theme.zoom, reduceMotion: Theme.reduceMotion,
            performanceMode: Theme.performanceMode,
            showLyrics: Settings.showLyrics, keepVisualizer: Settings.performanceKeepVisualizer,
            hasSession: Player.hasSession, canControl: Player.canControl, playing: Player.isPlaying,
            shuffle: Player.shuffle, repeatMode: Player.repeatMode,
            enabled: Visualizer.enabled, style: Visualizer.style,
            shape: Visualizer.shape, band: Visualizer.band,
            nowPlayingVisible: Visualizer.nowPlayingVisible, libraryBarVisible: Visualizer.libraryBarVisible,
            cover: {base: Theme.cover.base, veil: Theme.cover.veil, ink: Theme.cover.ink,
                glow: Theme.cover.glow, inkDeep: Theme.cover.inkDeep,
                veilOpacity: Theme.cover.veilOpacity, scrimOpacity: Theme.cover.scrimOpacity}
        }
    }
    function init() {
        Theme.mode = "dark"
        Theme.zoom = 1
        Theme.reduceMotion = true
        Theme.performanceMode = false
        Settings.performanceKeepVisualizer = false
        Settings.showLyrics = false
        Player.hasSession = true
        Player.canControl = true
        Player.isPlaying = true
        Player.shuffle = false
        Player.repeatMode = 0
        Visualizer.enabled = true
        Visualizer.style = "normal"
        Visualizer.shape = "bars"
        Visualizer.band = true
        Theme.cover.scrimOpacity = 0
    }
    function cleanupTestCase() {
        Theme.mode = savedState.mode
        Theme.zoom = savedState.zoom
        Theme.reduceMotion = savedState.reduceMotion
        Theme.performanceMode = savedState.performanceMode
        Settings.showLyrics = savedState.showLyrics
        Settings.performanceKeepVisualizer = savedState.keepVisualizer
        Player.hasSession = savedState.hasSession
        Player.canControl = savedState.canControl
        Player.isPlaying = savedState.playing
        Player.shuffle = savedState.shuffle
        Player.repeatMode = savedState.repeatMode
        Visualizer.enabled = savedState.enabled
        Visualizer.style = savedState.style
        Visualizer.shape = savedState.shape
        Visualizer.band = savedState.band
        Visualizer.setSurfaces(savedState.nowPlayingVisible, savedState.libraryBarVisible)
        for (const key of Object.keys(savedState.cover)) Theme.cover[key] = savedState.cover[key]
    }
    function make(component, properties) {
        const item = createTemporaryObject(component, test, properties || {})
        verify(item !== null)
        wait(0)
        return item
    }
    function near(actual, expected, message) {
        verify(Math.abs(actual - expected) < 1e-6,
               (message || "coordinate") + ": " + actual + " ~= " + expected)
    }
    function backgroundPoint(np) {
        const cover = np.coverArt
        for (let y = Math.ceil(np.height * Theme.nowPlayingClicks.backdropSplit); y < np.height; y += 4)
            for (let x = 2; x < np.width; x += 8) {
                if (!np.freeAt(x, y)) continue
                const q = cover.mapFromItem(np, x, y)
                if (q.x < -Theme.space.lg || q.y < -Theme.space.lg
                    || q.x > cover.width + Theme.space.lg || q.y > cover.height + Theme.space.lg)
                    return Qt.point(x, y)
            }
        return null
    }
    function reactiveLoader(np) {
        // The Loader is parented to CoverArt.ringSlot (no production objectName needed).
        for (const child of np.vizRing.children)
            if (child.item !== undefined && child.sourceComponent !== undefined) return child
        return null
    }
    function palette() {
        return {base: Player.paletteBase, glow: Player.paletteGlow,
            veilDark: Player.paletteVeilDark, veilLight: Player.paletteVeilLight,
            inkDark: Player.paletteInkDark, inkLight: Player.paletteInkLight,
            veilDarkOpacity: Player.paletteVeilDarkOpacity,
            veilLightOpacity: Player.paletteVeilLightOpacity}
    }

    function test_drift_covers_all_extremes_data() {
        return [{tag: "topLeft", sx: -1, sy: -1}, {tag: "topRight", sx: 1, sy: -1},
                {tag: "bottomLeft", sx: -1, sy: 1}, {tag: "bottomRight", sx: 1, sy: 1}]
    }
    function test_drift_covers_all_extremes(data) {
        const bg = make(backgroundComponent)
        const room = findChild(bg, "driftRoom")
        const canvas = findChild(bg, "driftCanvas")
        verify(room !== null)
        verify(canvas !== null)
        compare(canvas.parent, room)
        verify(bg.clip)
        compare(bg.driftOn, false)
        room.scale = 1
        room.x = data.sx * bg.width * Theme.nowPlaying.driftShift
        room.y = data.sy * bg.height * Theme.nowPlaying.driftShift
        const tl = canvas.mapToItem(bg, 0, 0)
        const br = canvas.mapToItem(bg, canvas.width, canvas.height)
        verify(tl.x <= 0 && tl.y <= 0, "canvas covers the top and left: " + tl)
        verify(br.x >= bg.width && br.y >= bg.height, "canvas covers the bottom and right: " + br)
        compare(bg.front.width, canvas.width)
        compare(bg.front.height, canvas.height)
    }
    function test_square_spot_cardinals_data() {
        return [{tag: "top", t: 0, x: 0, y: -1, nx: 0, ny: -1},
                {tag: "right", t: 0.25, x: 1, y: 0, nx: 1, ny: 0},
                {tag: "bottom", t: 0.5, x: 0, y: 1, nx: 0, ny: 1},
                {tag: "left", t: 0.75, x: -1, y: 0, nx: -1, ny: 0}]
    }
    function test_square_spot_cardinals(data) {
        const h = 100
        for (const rc of [0, 20]) {
            const spot = Viz.squareSpot({}, data.t, h, rc)
            near(spot.x, data.x * h)
            near(spot.y, data.y * h)
            near(spot.nx, data.nx)
            near(spot.ny, data.ny)
        }
    }
    function test_square_spot_rounded_corners() {
        const h = 100, rc = 20, half = h - rc
        const quarter = 2 * half + Math.PI * rc / 2
        const centres = [Qt.point(half, -half), Qt.point(half, half),
                         Qt.point(-half, half), Qt.point(-half, -half)]
        for (let q = 0; q < 4; ++q)
            for (const fraction of [0.1, 0.5, 0.9]) {
                // Sample within each corner arc, rather than the straight sides.
                const t = (q + (half + fraction * Math.PI * rc / 2) / quarter) / 4
                const spot = Viz.squareSpot({}, t, h, rc)
                const dx = spot.x - centres[q].x, dy = spot.y - centres[q].y
                near(Math.sqrt(dx * dx + dy * dy), rc, "corner radius")
                near(spot.nx, dx / rc, "outward normal x")
                near(spot.ny, dy / rc, "outward normal y")
            }
    }
    function test_square_into_stays_outside_and_closes() {
        const inner = 100, outer = 160, m = 96, cx = 7, cy = -11
        // With sharp corners the contour must stay outside the whole inner square.
        const out = Viz.squareInto([], [0, 0.2, 0.8, 1.2], cx, cy, inner, outer, m, 0, 1, 0)
        compare(out.length, m + 1)
        for (const point of out) {
            verify(isFinite(point.x) && isFinite(point.y))
            verify(Math.max(Math.abs(point.x - cx), Math.abs(point.y - cy)) >= inner - 1e-6,
                   "point outside the inner square: " + point)
        }
        near(out[0].x, out[m].x)
        near(out[0].y, out[m].y)
        const first = out[0]
        compare(Viz.squareInto(out, [0, 0, 0, 0], cx, cy, inner, outer, m, 0, 1, 0), out)
        compare(out[0], first)
        compare(out.length, m + 1)
        for (const point of out)
            near(Math.max(Math.abs(point.x - cx), Math.abs(point.y - cy)), inner, "silent outline")
    }

    function test_background_cycles_band_data() {
        return [{tag: "square", style: "normal"}, {tag: "circular", style: "circular"}]
    }
    function test_background_cycles_band(data) {
        Visualizer.style = data.style
        const np = make(npComponent)
        const p = backgroundPoint(np)
        verify(p !== null, "Now Playing has free background")
        for (const expected of [{shape: "wave", band: true}, {shape: "wave", band: false},
                                {shape: "bars", band: true}]) {
            mouseClick(np, p.x, p.y)
            compare(Visualizer.enabled, true)
            compare(Visualizer.style, data.style)
            compare(Visualizer.shape, expected.shape)
            compare(Visualizer.band, expected.band)
            compare(np.bandShown, expected.band)
            compare(np.coverReactive, true)
        }
    }
    function test_background_enables_visualizer_with_bars() {
        Visualizer.enabled = false
        Visualizer.band = false
        Visualizer.shape = "wave"
        Visualizer.style = "circular"
        const np = make(npComponent)
        const p = backgroundPoint(np)
        verify(p !== null)
        mouseClick(np, p.x, p.y)
        compare(Visualizer.enabled, true)
        compare(Visualizer.band, true)
        compare(Visualizer.shape, "bars")
        compare(Visualizer.style, "circular")
        compare(np.bandShown, true)
    }
    function test_cover_click_changes_only_style_data() {
        return [{tag: "on", enabled: true}, {tag: "off", enabled: false}]
    }
    function test_cover_click_changes_only_style(data) {
        Visualizer.enabled = data.enabled
        const np = make(npComponent)
        const button = findChild(np, "npCoverButton")
        verify(button.enabled)
        compare(button.Accessible.name, qsTr("Carátula: cambiar a circular"))
        mouseClick(np.coverArt)
        compare(Visualizer.style, "circular")
        compare(np.circular, true)
        compare(button.Accessible.name, qsTr("Carátula: cambiar a cuadrada"))
        mouseClick(np.coverArt)
        compare(Visualizer.style, "normal")
        compare(np.circular, false)
        compare(Visualizer.enabled, data.enabled)
        compare(Visualizer.band, true)
        compare(Visualizer.shape, "bars")
    }
    function test_cover_reactive_in_both_modes_data() {
        return [{tag: "square", style: "normal"}, {tag: "circular", style: "circular"}]
    }
    function test_cover_reactive_in_both_modes(data) {
        Visualizer.style = data.style
        const np = make(npComponent)
        const loader = reactiveLoader(np)
        verify(loader !== null)
        tryVerify(function () { return loader.item !== null })
        const ring = loader.item
        compare(ring.square, !np.circular)
        compare(ring.innerRadius, np.coverArt.artSize / 2)
        compare(ring.cornerRadius, np.coverArt.artCorner)
        compare(np.coverArt.frame, !np.circular)
        near(np.coverArt.artSize, np.coverArt.width * (np.circular
             ? Theme.nowPlaying.cookieRatio : Theme.nowPlaying.squareArtRatio))
        Visualizer.band = false
        compare(loader.item, ring, "turning off the lower band keeps the cover reactive")
        Visualizer.enabled = false
        tryCompare(loader, "item", null)
        compare(loader.active, false)
        compare(np.coverArt.frame, false)
        Visualizer.enabled = true
        tryVerify(function () { return loader.item !== null })
        compare(loader.item.square, !np.circular)
    }

    function test_player_bar_shuffle_reflects_mock() {
        const bar = make(barComponent)
        const button = findChild(bar, "playerBarShuffle")
        verify(button !== null)
        compare(button.active, false)
        mouseClick(button)
        compare(Player.shuffle, true)
        compare(button.active, true)
        mouseClick(button)
        compare(Player.shuffle, false)
        compare(button.active, false)
        Player.shuffle = true
        compare(button.active, true, "reflects changes from the backend too")
        Player.canControl = false
        verify(!button.enabled)
        mouseClick(button)
        compare(Player.shuffle, true)
    }
    function test_player_bar_repeat_reflects_mock() {
        const bar = make(barComponent)
        const button = findChild(bar, "playerBarRepeat")
        verify(button !== null)
        compare(button.active, false)
        for (const mode of [1, 2, 0]) {
            mouseClick(button)
            compare(Player.repeatMode, mode)
            compare(button.active, mode > 0)
            compare(button.iconSource, Qt.resolvedUrl("../../icons/" + (mode === 2 ? "repeat_one.svg" : "repeat.svg")))
        }
        Player.repeatMode = 2
        compare(button.active, true)
        compare(button.iconSource, Qt.resolvedUrl("../../icons/repeat_one.svg"))
        Player.canControl = false
        verify(!button.enabled)
        mouseClick(button)
        compare(Player.repeatMode, 2)
    }
    function test_scrims_follow_palette_and_visibility() {
        Settings.showLyrics = true
        // The text panels only show with the background still (user 2026-10-10).
        const drift = Settings.drift
        Settings.drift = false
        const np = make(npComponent)
        const controls = findChild(np, "npControlsScrim")
        const lyrics = findChild(np, "npLyricsScrim")
        verify(controls !== null)
        verify(lyrics !== null)
        for (const scrim of [controls, lyrics]) {
            compare(scrim.visible, false)
            compare(scrim.opacity, 0)
        }
        const p = palette()
        p.scrimDarkOpacity = 0.4
        p.scrimLightOpacity = 0.2
        Theme.setCoverPalette(p)
        compare(Theme.cover.scrimOpacity, 0.4)
        for (const scrim of [controls, lyrics]) {
            tryCompare(scrim, "visible", true)
            near(scrim.opacity, 0.4, "scrim opacity")
            verify(Qt.colorEqual(scrim.color, Theme.cover.veil))
        }
        Settings.showLyrics = false
        tryCompare(lyrics, "visible", false)
        compare(controls.visible, true)
        Player.hasSession = false
        compare(controls.visible, false)
        Theme.mode = "light"
        Theme.setCoverPalette(p)
        compare(Theme.cover.scrimOpacity, 0.2)
        delete p.scrimLightOpacity
        Theme.setCoverPalette(p)
        compare(Theme.cover.scrimOpacity, 0)
        Theme.mode = "dark"
        delete p.scrimDarkOpacity
        Theme.setCoverPalette(p)
        compare(Theme.cover.scrimOpacity, 0)
        Settings.drift = drift
    }
    // The hypnotic lights only use the cover's own colours and follow the theme (user 2026-10-10).
    function test_hypnotic_lights_follow_cover_and_theme() {
        const mode = Theme.mode
        const glow = Theme.cover.glow, base = Theme.cover.base
        Theme.cover.glow = "#C9A227"      // gold
        Theme.cover.base = "#8A8A8A"      // grey
        const bg = createTemporaryObject(backgroundComponent, test)
        Theme.mode = "dark"
        const dark = bg.blobColors.map(c => c.hslLightness)
        for (const c of bg.blobColors)
            verify(Math.abs(c.hslHue - Theme.cover.glow.hslHue) < 0.02 || c.hslSaturation < 0.05,
                   "a light took a hue the cover does not have")
        Theme.mode = "light"
        const light = bg.blobColors.map(c => c.hslLightness)
        for (let i = 0; i < dark.length; ++i)
            verify(light[i] > dark[i] + 0.15, "light theme lights must be clearly lighter")
        Theme.mode = mode
        Theme.cover.glow = glow
        Theme.cover.base = base
    }

    // T10: with the cover's five colours, every light keeps a hue the cover has (greys exempt).
    function test_hypnotic_lights_use_the_five_cover_colours() {
        const mode = Theme.mode
        const saved = Theme.cover.colors
        const cover = ["#8A8A8A", "#C9A227", "#6E6E6E", "#E0C050", "#3A3A3A"]   // grey and gold only
        Theme.cover.colors = cover
        const bg = createTemporaryObject(backgroundComponent, test)
        for (const m of ["dark", "light"]) {
            Theme.mode = m
            compare(bg.blobColors.length, 6)
            for (const c of bg.blobColors) {
                if (c.hslSaturation < 0.05)
                    continue
                let present = false
                for (const k of cover) {
                    const q = Qt.color(k)
                    if (q.hslSaturation < 0.05)
                        continue
                    const d = Math.abs(c.hslHue - q.hslHue)
                    if (Math.min(d, 1 - d) < 0.02)
                        present = true
                }
                verify(present, "a light took a hue the cover does not have: " + c)
            }
        }
        Theme.mode = mode
        Theme.cover.colors = saved
    }
    // T1: the palette carries the veil for the moving background; never below the static one.
    function test_moving_veil_follows_palette_and_never_drops_below_static() {
        const p = palette()
        p.veilDarkOpacity = 0.4
        p.veilDarkMovingOpacity = 0.7
        p.colors = ["#112233", "#445566"]
        Theme.mode = "dark"
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilMovingOpacity, 0.7)
        compare(Theme.cover.colors.length, 2)
        p.veilDarkMovingOpacity = 0.1
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilMovingOpacity, 0.4)
        delete p.veilDarkMovingOpacity
        Theme.setCoverPalette(p)
        compare(Theme.cover.veilMovingOpacity, 0.4)
    }
    // T1: still background uses the static veil; moving uses the higher one (no animation here:
    // reduceMotion/coverFade is not zero, so settle with tryCompare).
    function test_veil_rises_only_while_the_background_moves() {
        const bg = createTemporaryObject(backgroundComponent, test, {veilOpacity: 0.3, veilMovingOpacity: 0.8})
        compare(bg.movingMix, 0)
        verify(!bg.driftOn)
        const layer = bg.front
        compare(layer.veilAlpha, 0.3)
        compare(layer.veilMovingAlpha, 0.8)
    }
}
