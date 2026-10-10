pragma Singleton
import QtQuick

// Velo design tokens. Single source of truth for color, type, space, shape, depth and motion.
// Rules:
//  - Components read tokens from here; no literal colors, sizes or durations elsewhere.
//  - Two worlds share these tokens: the cover-drenched Now Playing surface (cover.*)
//    and the neutral library/settings surfaces (surface*, text*, accent).
//  - cover.* colors are computed in Rust once per track (palette extraction + contrast
//    clamping to >= 4.5:1 against the veil) and pushed in through setCoverPalette().
//  - Light/dark switching flips `dark`; the visible transition is a window-level
//    snapshot crossfade (opacity only), never per-property ColorAnimations.
QtObject {
    id: theme

    // ---------------------------------------------------------------- mode
    // Settings → Appearance writes `mode`; `systemDark` is fed from Qt.styleHints.
    property string mode: "system"          // "system" | "light" | "dark"
    // Writable so the app can apply a Windows light/dark change after taking the crossfade
    // snapshot (Main.qml); previews keep this live binding.
    property bool systemDark: Qt.styleHints.colorScheme === Qt.ColorScheme.Dark
    readonly property bool dark: mode === "dark" || (mode === "system" && systemDark)
    property bool reduceMotion: false       // Settings → Appearance
    property bool performanceMode: false    // Settings → Appearance → "Modo rendimiento"
    property bool pauseInBackground: false  // Settings → Appearance: stop effects when the window loses focus
    property bool drift: true               // Settings → Background: slow movement of the blurred art
    property bool windowVisible: true
    property bool windowFocused: true
    property bool playing: true
    readonly property bool highContrast: Qt.styleHints.accessibility.contrastPreference === Qt.HighContrast
    readonly property bool effectsAllowed: windowVisible && playing
        && (windowFocused || !pauseEffectsInBackground)
    // Effective switches: Modo rendimiento forces effects off / pause on, whatever the user chose.
    readonly property bool pauseEffectsInBackground: pauseInBackground || performanceMode
    readonly property bool driftActive: drift && !performanceMode && effectsAllowed
    // Settings → Background → «Teñir la interfaz»: the cover palette colors the Now Playing
    // controls (play disc, active tools, visualizer). Off: the same controls in neutral ink.
    property bool tintUi: true

    // ---------------------------------------------------------------- zoom (U1)
    // Ajustes › Apariencia › Tamaño de la interfaz (Settings.uiZoom; Ctrl+Más / Ctrl+Menos / Ctrl+0).
    // Main binds `zoom`. Every length token below (type, space, radius, icons, sizes, panels…)
    // goes through px(), so a zoom change lays the whole UI out again at the new size: text and
    // icons are rasterized at their final size (crisp), hit testing and the native title-bar
    // test keep working, and the window minimum follows. Nothing is a scaled texture.
    property real zoom: 1.0
    readonly property real zoomMin: 0.75
    readonly property real zoomMax: 1.5
    readonly property real zoomFactor: Math.max(zoomMin, Math.min(zoomMax, isFinite(zoom) && zoom > 0 ? zoom : 1))
    function px(n) { return Math.round(n * zoomFactor) }
    // Next zoom on the 0.1 grid: dir 1 up, -1 down, 0 back to 100 %. Clamped to [zoomMin, zoomMax].
    function nextZoom(current, dir) {
        if (dir === 0) return 1
        const tenth = (isFinite(current) && current > 0 ? current : 1) * 10
        const next = dir > 0 ? (Math.floor(tenth + 1e-6) + 1) / 10 : (Math.ceil(tenth - 1e-6) - 1) / 10
        return Math.max(zoomMin, Math.min(zoomMax, Math.round(next * 100) / 100))
    }

    // --------------------------------------------------- neutral world (library, settings)
    // Faintly green-tinted neutrals so the accent belongs to the ground instead of sitting on it.
    readonly property color bg:        dark ? "#0E0F0F" : "#F5F7F6"
    readonly property color surface1:  dark ? "#151716" : "#FFFFFF"   // sidebar, panels
    readonly property color surface2:  dark ? "#1C1F1E" : "#EEF1EF"   // cards, rows on hover
    readonly property color surface3:  dark ? "#262A28" : "#E2E6E3"   // pressed, segmented track
    readonly property color hairline:  dark ? Qt.rgba(1, 1, 1, 0.07) : Qt.rgba(0, 0, 0, 0.08)   // dividers only (decorative)
    // Edges that identify a control: >= 3:1 (WCAG 1.4.11) on bg and surface1/2/3, both modes.
    readonly property color controlBorder: highContrast ? textSecondary : dark ? "#6E7672" : "#767D79"
    readonly property color scrim:     Qt.rgba(0, 0, 0, dark ? 0.55 : 0.32)

    readonly property color text:          dark ? "#F1F4F2" : "#111413"   // 17.3:1 / 17.2:1 on bg
    readonly property color textSecondary: dark ? "#A7AFAB" : "#4B5450"   // 8.6:1 / 7.3:1
    readonly property color textTertiary:  dark ? "#8C9591" : "#5C6561"   // 6.2:1 / 5.6:1 (>= 4.7 on every surface, surface3 included)
    readonly property color textDisabled:  dark ? "#565D5A" : "#A2A9A5"   // decorative only

    // Accent: Velo green. Not Spotify's #1ED760; a cooler, bluer green so we never wear their dress.
    // Usable as text too: >= 4.5:1 on every surface in both modes (dark 7.7 on surface3; light
    // 5.5 / 5.9 / 5.2 / 4.7 on bg / surface1 / surface2 / surface3, and 5.2 on accentSoft).
    readonly property color accent:        dark ? "#3CD68A" : "#0A733F"   // fills, toggles, sliders, selected text
    readonly property color accentHover:   dark ? "#5BE29F" : "#085F33"
    readonly property color accentInk:     dark ? "#04140B" : "#FFFFFF"   // text/icons on accent (10.1:1 / 5.9:1)
    readonly property color accentSoft:    dark ? Qt.rgba(0.235, 0.84, 0.54, 0.14) : Qt.rgba(0.039, 0.451, 0.247, 0.10)
    readonly property color danger:        dark ? "#FF6B6B" : "#C42B2B"
    readonly property color focusRing:     accent

    // ---------------------------------------------------- cover world (Now Playing)
    // Default teal palette until the first track arrives.
    readonly property QtObject cover: QtObject {
        readonly property int blurSourceSize: 96 // Rust's cached background; GPU scales this texture
        readonly property int tipDelay: theme.dj.tipDelay
        property color base:     "#2E6F86"   // dominant hue, raw
        property color veil:     theme.dark ? "#14323D" : "#D7E8EE"   // overlay over the blurred art
        // Veil alpha over the blurred art. Rust picks it per track (with the ink) so the ink keeps
        // >= 4.5:1 against the real composite; setCoverPalette() replaces this default.
        property real  veilOpacity: 1.0
        property color ink:      theme.dark ? "#F4FAFC" : "#0D2129"   // primary text on veil
        readonly property color inkSoft: theme.coverInkSecondary // compatibility alias for secondary ink
        property color inkFaint: Qt.rgba(ink.r, ink.g, ink.b, 0.34) // decorative tracks and disabled controls only
        property color glow:     "#7CC9E6"   // visualizer, cover blob (vibrant swatch)
        property color pill:     Qt.rgba(ink.r, ink.g, ink.b, theme.dark ? 0.12 : 0.10) // tool pill, active lyric
        property color pillHover: Qt.rgba(ink.r, ink.g, ink.b, theme.dark ? 0.20 : 0.16)
        property color inkDeep:  "#0D2129"   // the palette's dark ink (light-mode text), tints the controls
        // Scalloped play disc and active tools: palette-tinted, or neutral without «Teñir la interfaz».
        readonly property color control:  theme.dark ? "#FFFFFF" : theme.tintUi ? inkDeep : theme.text
        readonly property color controlInk: theme.dark ? (theme.tintUi ? inkDeep : theme.bg) : "#FFFFFF"
    }

    // Called from Main.qml when the backend publishes a new palette (strings "#RRGGBB").
    function setCoverPalette(p) {
        cover.base = p.base
        cover.veil = dark ? p.veilDark : p.veilLight
        cover.ink = dark ? p.inkDark : p.inkLight
        cover.glow = p.glow
        cover.inkDeep = p.inkLight
        // R5: veil alpha per track; Rust guarantees contrast against the composite cover + veil.
        // Opaque when a caller does not pass it (contrast stays safe).
        const opacity = dark ? p.veilDarkOpacity : p.veilLightOpacity
        cover.veilOpacity = isFinite(opacity) ? Math.max(0, Math.min(1, opacity)) : 1
    }

    // ---------------------------------------------------------------- type
    // Figtree (OFL, bundled in qml/fonts). Geometric, warm, heavy weights hold up as display
    // over busy blurred art, which is what the references ask of their title and lyrics.
    readonly property FontLoader _figtree: FontLoader { source: Qt.resolvedUrl("../fonts/Figtree-Variable.ttf") }
    readonly property string family: _figtree.status === FontLoader.Ready ? _figtree.name : "Segoe UI Variable"

    // Size scale (px at 100 %) and the role each size plays. Not a fixed ratio: 1–2 px steps
    // between the reading sizes (11–16), then wide jumps for headings and display (20–56).
    readonly property QtObject type: QtObject {
        readonly property int micro:    theme.px(11)   // timestamps, counters (tabular figures)
        readonly property int caption:  theme.px(12)   // album line, helper text, column headers
        readonly property int body:     theme.px(14)   // rows, settings labels
        readonly property int title:    theme.px(16)   // row titles, section labels
        readonly property int heading:  theme.px(20)   // settings page title, dialog titles
        readonly property int lyric:    theme.px(28)   // lyric lines (active and inactive share size; only ink/weight change)
        readonly property int display:  theme.px(34)   // Now Playing track title
        readonly property int hero:     theme.px(56)   // playlist header name

        readonly property int regular:  Font.Normal
        readonly property int medium:   Font.Medium
        readonly property int semibold: Font.DemiBold
        readonly property int bold:     Font.Bold
        readonly property int heavy:    Font.ExtraBold

        // Letter spacing in px at the given size (tracking tightens as size grows; floor -0.04em).
        // Thresholds are 100 % sizes: divide by the zoom so a role keeps its tracking at any zoom.
        function tracking(size) {
            const base = size / theme.zoomFactor
            return base >= 48 ? -size * 0.035 : base >= 28 ? -size * 0.02 : base >= 16 ? -size * 0.005 : 0
        }
        readonly property real lyricLineHeight: 1.18
        readonly property real bodyLineHeight: 1.4
    }
    // Use for every time/number column so digits do not jitter while counting.
    readonly property var tabularFeatures: ({ "tnum": 1 })

    // --------------------------------------------------------------- space
    // 4 px base. Tight inside groups (xs/sm), generous between groups (xl+).
    readonly property QtObject space: QtObject {
        readonly property int xxs: theme.px(2)
        readonly property int xs: theme.px(4)
        readonly property int sm: theme.px(8)
        readonly property int md: theme.px(12)
        readonly property int lg: theme.px(16)
        readonly property int xl: theme.px(24)
        readonly property int xxl: theme.px(32)
        readonly property int xxxl: theme.px(48)
        readonly property int gutter: theme.px(28)        // window content inset (matches references)
    }

    // --------------------------------------------------------------- shape
    readonly property QtObject radius: QtObject {
        readonly property int xs: theme.px(4)              // focus ring inner, tiny chips
        readonly property int sm: theme.px(6)              // track-row thumbnails, inputs
        readonly property int md: theme.px(10)             // buttons, segmented items, rows
        readonly property int lg: theme.px(14)             // cards, settings panel, popovers
        readonly property int xl: theme.px(18)             // square cover art (Now Playing)
        readonly property int segment: theme.px(12)        // VSegmented track
        readonly property int segmentInner: theme.px(8)    // its pill: segment minus the 4 px inset (concentric)
        readonly property int field: theme.px(10)
        // Pills: use height / 2 at the call site.
    }

    // Scalloped "cookie" outline used by the play button and circular cover.
    readonly property QtObject scallop: QtObject {
        readonly property int lobesPlay: 9
        readonly property int lobesCover: 16
        readonly property real depth: 0.055      // lobe amplitude as a fraction of the radius
    }

    // Line weights.
    // Hairline and focus ring stay 1 / 2 px at any zoom (crisp edges); the rest follow the zoom.
    readonly property QtObject stroke: QtObject {
        readonly property real hairline: 1
        readonly property real progress: 3 * theme.zoomFactor       // progress/volume track
        readonly property real progressWave: 3.5 * theme.zoomFactor
        readonly property real icon: 2 * theme.zoomFactor           // Material Symbols Rounded, weight 500 equivalent
        readonly property real focus: 2
    }

    // --------------------------------------------------------------- depth
    // For QtQuick.Effects RectangularShadow (SDF, single draw call, no blur passes).
    // The main window shadow and corners come from DWM on Windows 11, not from QML.
    readonly property QtObject elevation: QtObject {
        readonly property var cover:   ({ offsetY: theme.px(18), blur: theme.px(48), spread: -theme.px(6), alpha: 0.38 })   // cover art lifting off the veil
        readonly property var popover: ({ offsetY: theme.px(10), blur: theme.px(28), spread: -theme.px(4), alpha: dark ? 0.50 : 0.18 })
        readonly property var raised:  ({ offsetY: theme.px(4),  blur: theme.px(12), spread: -theme.px(2), alpha: dark ? 0.40 : 0.12 })
        readonly property bool dark: theme.dark
        readonly property color popoverShadow: Qt.rgba(0, 0, 0, popover.alpha)   // RectangularShadow color of popovers and dialogs
    }

    // --------------------------------------------------------------- icons
    readonly property QtObject icon: QtObject {
        readonly property int sm: theme.px(16)
        readonly property int md: theme.px(20)
        readonly property int lg: theme.px(24)
        readonly property int transport: theme.px(22)      // prev / next / shuffle / repeat
        readonly property int play: theme.px(26)           // glyph inside the scalloped disc
    }

    // --------------------------------------------------------------- sizes
    readonly property QtObject size: QtObject {
        readonly property int titleBar: theme.px(44)
        readonly property int toolPill: theme.px(32)
        readonly property int iconButton: theme.px(32)
        readonly property int minimumHitTarget: 32    // screen pixels: still 32 at zoom 0.75
        readonly property int playDisc: theme.px(52)
        readonly property int coverMin: theme.px(200)
        readonly property int coverMax: theme.px(300)
        readonly property int sidebar: theme.px(232)
        readonly property int sidebarCollapsed: theme.px(72)
        // G4: the library sidebar can be resized by dragging its right edge (default `sidebar`).
        // Below `sidebarCompactBelow` its list switches to the compact rows on its own.
        readonly property int sidebarMin: theme.px(200)
        readonly property int sidebarMax: theme.px(420)
        readonly property int sidebarCompactBelow: theme.px(216)
        readonly property int trackRow: theme.px(56)
        readonly property int playerBar: theme.px(80)
        // Main window at 100 %: first size and minimum (the minimum follows the zoom, so a zoomed
        // layout never gets squeezed below what it needs; Main caps it to the screen). The first
        // size has no px() on purpose: it is set before the saved zoom applies, and afterwards
        // the window keeps whatever size the user gives it.
        readonly property int windowWidth: 1100
        readonly property int windowHeight: 680
        readonly property int windowMinWidth: theme.px(860)
        readonly property int windowMinHeight: theme.px(560)
    }

    // --------------------------------------------------------------- motion
    // Shared motion values: all
    // components only read these tokens.
    //  - `scale` multiplies every UI duration. "Reducir movimiento" and "Modo rendimiento" set it
    //    to 0, so every Behavior/Animator bound to these tokens becomes an instant change.
    //  - `themeFade`, `viewFade` and `coverFade` are fades, not movement: "Reducir movimiento" keeps
    //    them (a crossfade does not trigger motion sensitivity; the view zoom is dropped instead);
    //    only "Modo rendimiento" sets them to 0.
    //  - Values unchanged from the provisional set: frequent things stay short (90–150 ms),
    //    occasional view changes 220–320 ms, the two crossfades longer because they bridge whole
    //    screens (window 280 ms) or a slow ambient background (cover 700 ms).
    // Easing curves are cubic beziers in QML's bezierCurve form [x1, y1, x2, y2, 1, 1]
    // (Easing.Bezier); they also work on Animators (render thread).
    readonly property QtObject motion: QtObject {
        readonly property real scale: theme.reduceMotion || theme.performanceMode || !theme.effectsAllowed ? 0 : 1
        readonly property int instant: 90 * scale     // press feedback
        readonly property int fast: 150 * scale       // hover, toggles
        readonly property int base: 220 * scale       // view state changes
        readonly property int slow: 320 * scale       // panels, lyric line handoff
        readonly property int themeFade: theme.performanceMode || !theme.effectsAllowed ? 0 : 280
        readonly property int viewFade: theme.performanceMode || !theme.effectsAllowed ? 0 : 220
        readonly property int coverFade: theme.performanceMode || !theme.effectsAllowed ? 0 : 700
        // U1: interface zoom (Ctrl+Más/Menos/0, Ajustes): the window at the old size crossfades
        // into the new layout. A fade, so Reducir movimiento keeps it; Modo rendimiento drops it.
        readonly property int zoomFade: theme.performanceMode || !theme.effectsAllowed ? 0 : 160
        readonly property int shakeStep: 50 * scale
        readonly property real viewZoomFrom: 0.98     // incoming view settles 0.98 → 1 over viewFade (skipped when scale is 0)
        readonly property var easeOut:   [0.16, 1, 0.3, 1, 1, 1]     // exponential out: entering, responding
        readonly property var easeInOut: [0.65, 0, 0.35, 1, 1, 1]    // on-screen movement A→B, crossfades
        readonly property var easeIn:    [0.7, 0, 0.84, 0, 1, 1]     // exits only
        readonly property var npEase: [0.25, 1, 0.5, 1, 1, 1]         // easings.net easeOutQuart: long, even ease-out for the choreography windows

        // S1-NP: Library ↔ Now Playing choreography (components/motion/NowPlayingMorph.qml +
        // views/NowPlaying.qml). A few times per session, so it is the generous one. Everything
        // reads one master progress `presence` (0 = folded into the mini player, 1 = at rest);
        // each element owns a window [start, end] of it. The exit plays the same windows backwards
        // (shorter), so a change of mind halfway reverses from wherever things are. The library
        // stays opaque under Now Playing the whole time (the room fades over it).
        // Reducir movimiento / Modo rendimiento: this view transition is instant.
        readonly property int npEnter: 950 * scale      // presence 0 → 1
        readonly property int npExit: 480 * scale       // presence 1 → 0 (Now Playing folds away)
        readonly property int npArtWait: 120            // longest wait for the big cover to decode before it flies (ms)
        readonly property var npWinBackdrop: [0, 0.6]   // library sinks while the blurred room blooms over it
        readonly property var npWinCover: [0, 0.7]      // cover flies out of the mini player and lands
        readonly property real npCoverLand: 0.85        // position and scale arrive together; last 15% is the landing pop
        readonly property real npCoverPop: 1.045        // the cover lands with a short pop to this scale
        readonly property var npWinRing: [0.36, 0.9]    // circular visualizer opens around the cookie
        readonly property var npWinBand: [0.4, 0.86]    // bottom band grows from its baseline
        readonly property var npWinTop: [0.5, 0.86]     // close disc and tool pill drop in
        readonly property var npWinLyrics: [0.42, 1]    // lyric lines rise, top line first
        readonly property real npLyricsStagger: 0.5     // share of the lyrics window spent staggering lines
        readonly property real npControlsStart: 0.3     // title, progress, transport, volume rise in turn
        readonly property real npControlsStep: 0.07
        readonly property real npControlsSpan: 0.36
        readonly property real npLibrarySink: 0.92      // the library recedes to this under Now Playing
        readonly property real npBloom: 1.15            // blurred background blooms from this scale
        readonly property real npCoverSpin: -8          // cover leaves the mini player turned by this (deg)
        readonly property int npControlsRise: theme.px(28)        // px
        readonly property int npLyricsRise: theme.px(56)          // px
        readonly property int npTopDrop: theme.px(14)             // px
        readonly property real npBandFrom: 0.05         // band height share at the start (never from nothing)
        readonly property real npRingFrom: 0.6          // ring scale at the start
        readonly property real npRingTurn: -45          // ring unwinds from this (deg)

        // S1-NP: cover art as a card under the pointer (CoverArt). The point under the cursor
        // sinks back; a spring follows the pointer and settles flat with a soft bounce on leave.
        // Off (flat) when `scale` is 0 (Reducir movimiento / Modo rendimiento).
        readonly property real tiltMax: 13              // deg at the edge of the art
        readonly property int tiltPerspective: theme.px(700)      // px from the eye to the screen (smaller = stronger)
        readonly property real tiltLift: 1.03           // the card rises a little while hovered
        readonly property int tiltSlop: theme.space.lg  // G4: hover keeps tilting this far past the (lifted) art
        readonly property real tiltShadowShift: theme.px(14)      // px the shadow slides away from the pointer
        readonly property real tiltGlare: 0.2           // specular highlight alpha under the pointer
        readonly property real tiltGlareRadius: 0.75    // fixed gradient, opacity only during hover
        readonly property real tiltSpring: 5            // SpringAnimation.spring
        readonly property real tiltDamping: 0.3         // SpringAnimation.damping (< 1: a small bounce)
        // Cover change (also the DJ's portrait ↔ album art): a crossfade, plus a slight settle.
        readonly property int artFade: theme.performanceMode || !theme.effectsAllowed ? 0 : 450
        readonly property real artFadeZoom: 0.97
    }

    // Inicio de sesión / primer uso: mismo lenguaje de las referencias.
    readonly property QtObject login: QtObject {
        readonly property int formWidth: theme.px(520)
        readonly property int markSize: theme.px(72)
        readonly property int markIcon: theme.px(34)
        readonly property int heroMark: theme.px(104)   // logo de la pantalla de inicio de sesión
        readonly property int heroGlyph: theme.px(50)
        readonly property int buttonHeight: theme.px(56)
        readonly property real successPulse: 1.08
        readonly property real exitScale: 1.04
        readonly property int shakeDistance: theme.space.sm
    }

    // R1: alert banner of the whole app (components/Toast.qml). Its countdown stops while the
    // pointer or the keyboard focus is on it; a banner with an action waits longer.
    readonly property QtObject toast: QtObject {
        readonly property int width: theme.login.formWidth + theme.space.xxxl
        readonly property int timeout: 5000          // not motion: unaffected by reduceMotion
        readonly property int actionTimeout: 10000
    }

    // R1: tooltip of icon-only buttons (IconButton) on hover or keyboard focus.
    readonly property QtObject tip: QtObject {
        readonly property int delay: 500             // before it shows (not motion)
        readonly property int gap: theme.space.sm    // distance to the button
        readonly property int padding: theme.space.sm
        readonly property int maxWidth: theme.px(320)
    }

    // R1: «Teñir la interfaz» (Settings.tintUi): the library ground takes a faint wash of the
    // playing cover's hue. One flat rectangle, no animation; light enough that every text pair
    // keeps its measured contrast.
    readonly property QtObject tint: QtObject {
        readonly property real alpha: theme.dark ? 0.07 : 0.05
    }

    // ------------------------------------------------------------ visualizer
    readonly property QtObject viz: QtObject {
        readonly property int bands: 48
        readonly property int bandHeight: theme.v1Viz.bandMaxHeight
    }

    // --- Integración de tokens (U2) ---
    // obsoleto (R1): compatibility alias for foreign files; use Theme.library.disabledOpacity.
    readonly property real disabledOpacity: library.disabledOpacity

    // Secondary ink over the cover veil (Rust checks these alphas against veil + blurred art).
    readonly property color coverInkSecondary: Qt.rgba(cover.ink.r, cover.ink.g, cover.ink.b, dark ? 0.62 : 0.8)
    readonly property color coverInkLyric:     Qt.rgba(cover.ink.r, cover.ink.g, cover.ink.b, dark ? 0.74 : 0.8)
    // The vibrant swatch glows on the dark veil but washes out on the light one: use the base hue there.
    // Without «Teñir la interfaz» it is drawn in the cover ink (neutral).
    readonly property color coverVizColor: !tintUi ? cover.ink : dark ? cover.glow : cover.base

    readonly property QtObject control: QtObject {
        readonly property int sliderKnobHeight: theme.icon.sm
        readonly property int toggleWidth: theme.px(46)
        readonly property int toggleHeight: theme.px(28)
        readonly property real toggleKnobInset: theme.space.xs - theme.stroke.hairline
    }

    readonly property QtObject panel: QtObject {
        readonly property int devicePopoverWidth: theme.px(320)
        readonly property int deviceRow: theme.px(52)
        readonly property int pickerWidth: theme.px(320)
        readonly property int pickerRow: theme.px(48)
        readonly property int pickerRows: 6                          // rows before the list scrolls
        readonly property int queueWidth: theme.px(340)
        // G4: the queue/Recientes panel can be resized by dragging its left edge.
        readonly property int queueMinWidth: theme.px(280)
        readonly property int queueMaxWidth: theme.px(520)
        readonly property int queueThumb: theme.px(40)
        readonly property int settingsAvatar: theme.px(56)
    }

    readonly property QtObject nowPlaying: QtObject {
        readonly property real leftColumnRatio: 0.46    // left column share of the width with lyrics
        readonly property real lyricsStartRatio: 0.5    // lyrics pill left edge
        readonly property real coverHeightRatio: 0.34   // cover side vs window height (clamped to coverMin..coverMax)
        readonly property real ringSlotRatio: 1.15      // circular slot (cookie + visualizer blob) vs square cover
        readonly property real cookieRatio: 0.69        // cookie diameter vs slot in circular mode
        readonly property real scallopDepthPlay: 0.09   // the 52 px disc needs deeper notches than the cover
        readonly property real waveLength: 22 * theme.zoomFactor           // wavy progress, px
        readonly property real waveAmplitude: 3 * theme.zoomFactor
        // --- R2 ---
        // Narrow windows: below this the cover gives way so the left column fits (560 px minimum).
        readonly property int coverFloor: theme.px(120)
        // Slow drift of the blurred room («Deriva lenta»): scale breathes between driftScaleMin
        // and driftScaleMax and the layer wanders by driftShift of its size (the minimum scale
        // keeps the edges out of sight); each axis has its own period so the path never visibly
        // repeats. Animators (render thread), only while playing and on screen.
        readonly property real driftScaleMin: 1.06
        readonly property real driftScaleMax: 1.1
        readonly property real driftShift: 0.025
        readonly property int driftScaleMs: 46000
        readonly property int driftXMs: 62000
        readonly property int driftYMs: 54000
        // Lyrics: after the user scrolls them, following the song resumes after this pause
        // (one-shot, never while paused).
        readonly property int lyricsResumeMs: 3500
        // Volume: how long the gesture's value is held waiting for the backend's echo.
        readonly property int volumeSettleMs: 1500
        // Mouse-wheel notch (Windows convention); touchpads send fractions of it.
        readonly property int wheelNotch: 120
        readonly property int deviceSkeletonRows: 2
        readonly property int deviceMaxRows: 6
    }

    readonly property QtObject vizStyle: QtObject {
        readonly property real alphaTop: 0.92           // band fill at the crest
        readonly property real alphaBottom: 0.38        // band fill at the base
        readonly property real alphaOuter: 0.9          // ring fill at the rim
        readonly property real alphaInner: 0.45
        readonly property real barGap: theme.space.xs
        readonly property real floor: 0.03              // band: silence still leaves a thin line
        readonly property int waveSubdivisions: 4       // interpolated points per band step
        readonly property int waveSmoothing: 2          // 1-2-1 passes across neighbour bands (wave only)
        readonly property real ringFloor: 0.08          // spokes
        readonly property real blobFloor: 0.3           // resting size of the blob
        readonly property int ringSamples: 144          // polygon points per turn
        readonly property real ringSpokeWidth: theme.space.xs
        // Bars follow the volume: height × (volume / 100) ^ volumeCurve (0 when muted). < 1 keeps
        // low volumes readable; the analysis itself stays pre-volume (V1b).
        readonly property real volumeCurve: 0.7
        // G4: the drawn level never leaves its box. Up to `knee` it is drawn as is; above, it
        // bends softly toward 1 (no flat plateau, never past the top edge).
        readonly property real knee: 0.8
        // G4: display-rate smoothing. When the bands arrive slower than the screen refreshes
        // (more than `lerpRatio` display frames apart), the drawing glides from one set to the
        // next over the measured gap (capped at `lerpMaxMs`): one short NumberAnimation per
        // update, nothing in between updates or with the music paused.
        readonly property real lerpRatio: 1.5
        readonly property int lerpMaxMs: 80
        readonly property real fallbackHz: 60   // Screen.refreshRate unknown (0)
    }

    // Window controls: macOS-style discs (fixed palette, same in light and dark) and the Windows caption buttons.
    // R1: also the sizes of both styles (formerly `Theme.caption`, which clashed with type.caption).
    readonly property QtObject windowControls: QtObject {
        readonly property var trafficClose:    ({ fill: "#FF5F57", ring: "#E2463F", glyph: "#4D0000" })
        readonly property var trafficMinimize: ({ fill: "#FEBC2E", ring: "#DFA023", glyph: "#995700" })
        readonly property var trafficZoom:     ({ fill: "#28C840", ring: "#1AAB29", glyph: "#006500" })
        readonly property color trafficIdle:     theme.dark ? Qt.rgba(1, 1, 1, 0.16) : Qt.rgba(0, 0, 0, 0.13)
        readonly property color trafficIdleRing: theme.dark ? Qt.rgba(1, 1, 1, 0.10) : Qt.rgba(0, 0, 0, 0.08)
        readonly property color closeHover: "#C42B1C"
        readonly property color closePress: "#C83C31"
        readonly property color closeInk: "#FFFFFF"
        readonly property int trafficDiameter: theme.px(12)
        readonly property int trafficGap: Math.max(theme.px(8), theme.size.minimumHitTarget - trafficDiameter)
        // The drawing stays 12 px at 100 %; disjoint click cells stay at least 32 screen px.
        // Up/down slop equals half the gap sideways, bringing the target to the window edge.
        readonly property real trafficHitSlop: trafficGap / 2
        readonly property real trafficRing: 0.5
        readonly property int trafficGlyph: theme.px(10)
        readonly property real pressDarken: 1.16
        readonly property int buttonWidth: theme.px(46)
        readonly property real hoverAlpha: 0.09
        readonly property real pressAlpha: 0.05
        readonly property real inactiveOpacity: 0.45
    }

    // --- Biblioteca (U1) ---
    // Library-only tokens (library pages, sidebar, player bar, login). Namespaced under
    // `library` so they never collide with tokens other tasks add for their own surfaces.
    readonly property QtObject library: QtObject {
        // Paint for "no fill" states (avoids the string "transparent" at call sites).
        readonly property color clearPaint: Qt.rgba(0, 0, 0, 0)
        // Whole-control opacity for disabled buttons, cards and fields. Disabled controls are
        // exempt from text contrast (WCAG 1.4.3) but must read as unavailable at a glance.
        readonly property real disabledOpacity: 0.42
        // Keyboard focus ring on accent fills (primary button, play disc): drawn inset by
        // stroke.focus so accent surrounds it on both sides. accentInk vs accent measures
        // 10.1:1 (dark) and 5.9:1 (light), above the 3:1 non-text minimum.
        readonly property color focusOnAccent: theme.accentInk
        // Sidebar tint over the real Windows Mica backdrop (no QML blur).
        readonly property real sidebarMicaOpacity: 0.78
        // Playlist / album / Me gusta header artwork.
        readonly property int headerCover: theme.size.coverMin - theme.space.xxl
        // Cards in grids and home shelves (art is square; the text block is size.trackRow tall).
        readonly property int cardWidth: theme.size.coverMin - theme.space.xxxl
        // Comfortable measure for explanatory paragraphs (~85 characters at type.body).
        readonly property int readableWidth: theme.size.sidebar * 3
        // Login form column.
        readonly property int loginFormWidth: theme.size.sidebar * 2
        // Search-as-you-type delay. Not motion: unaffected by reduceMotion.
        readonly property int searchDebounce: 300
        // Smallest scrollbar thumb as a fraction of the track (long lists stay grabbable).
        readonly property real scrollMinimumFraction: 0.05
        // R4: list pages, track rows and their menus.
        readonly property int creditsMaxRows: 7                       // credits rows before the list scrolls
        readonly property int gridArtStep: theme.space.xl             // grid cards grow in steps: art decodes per step, not per pixel of a resize
        readonly property real heroTitleLines: 1.8                    // page titles longer than this many hero lines use type.display
        readonly property int skeletonRows: 6                         // static placeholder rows while a list first loads
        readonly property color contextMenuShadow: Qt.rgba(0, 0, 0, theme.elevation.popover.alpha)
        readonly property int cardBleed: theme.space.sm
        readonly property int chipHeight: theme.size.iconButton
        readonly property int bioExpandedLines: 1000
        readonly property int playlistNameMax: 100
        readonly property int playlistDescriptionMax: 300
        // R3: Inicio, barra lateral, barra de reproducción y Ajustes (s10).
        // Shadow of menus and tooltips (same elevation as the audio popovers).
        readonly property color popoverShadow: Qt.rgba(0, 0, 0, theme.elevation.popover.alpha)
        // Shelf arrows of Inicio: round buttons at the ends of a shelf, shown on hover.
        readonly property int shelfArrow: theme.size.iconButton
        // Share of a shelf's width one arrow click advances (one "page" of cards).
        readonly property real shelfPage: 0.9
        // Hover play disc of cards (MediaCard), inset from the art's bottom-right corner.
        readonly property int cardPlay: theme.size.iconButton + theme.space.sm
        // Static placeholders of Inicio while its first data arrives (no shimmer, nothing runs).
        readonly property int skeletonTiles: 4
        readonly property int skeletonCards: 5
        // Volume: how long the knob holds the user's value waiting for the backend (not motion).
        readonly property int volumeSettle: 1500
        // Spotify Connect device name length (Ajustes › Reproducción).
        readonly property int deviceNameMax: 64
        // Ajustes › Visualizador › Sensibilidad, in percent.
        readonly property int sensitivityMin: 25
        readonly property int sensitivityMax: 200
        readonly property int sensitivityStep: 5
        // Equalizer columns never get narrower than this; below it the bands scroll sideways.
        readonly property int eqColumnCompact: theme.px(28)
        // Ajustes index: rail of sections at the left (as in ref_02), shown when the page is at
        // least `settingsRailMinPage` wide; narrower pages get a row of chips instead.
        readonly property int settingsRail: theme.px(84)
        readonly property int settingsRailItem: theme.px(60)
        readonly property int settingsRailMinPage: theme.px(620)
        // Artwork decode size steps (grids resize continuously; decode only per step).
        readonly property int artDecodeStep: theme.space.xxxl
        readonly property int wheelScroll: theme.size.trackRow
        // Control tracks need >= 3:1 against the neutral surfaces in both themes.
        readonly property color controlTrack: theme.textTertiary
    }
    // --- R1b ---
    // obsoleto (R1): the same object as windowControls (NowPlaying.qml still reads
    // caption.buttonWidth); remove once nothing reads it.
    readonly property QtObject caption: windowControls
    readonly property QtObject sliderInput: QtObject {
        readonly property int volumeDefault: 50
        readonly property int volumeStep: 5
        readonly property int volumePage: 10
        readonly property int seekStepMs: 5000
        readonly property int seekPageMs: 30000
    }

    // --- V1 ---
    readonly property QtObject v1Viz: QtObject {
        readonly property int bandMaxHeight: theme.px(180)
    }

    // --- U3 ---
    readonly property QtObject audioOutput: QtObject {
        readonly property int maxRows: 6
        readonly property int volumePanelHeight: theme.size.trackRow * 2 + theme.space.lg * 2
        // G4: the volume panel stays open while the pointer is within this much of the button
        // or the panel (a small slop, so a slightly loose hand does not fold it).
        readonly property int volumeHoverSlop: theme.space.sm
        readonly property int cardBleed: theme.library.cardBleed // obsoleto: use library.cardBleed
        readonly property color shadow: Qt.rgba(0, 0, 0, theme.elevation.popover.alpha)
    }

    // --- V1b ---
    readonly property QtObject v1bViz: QtObject {
        readonly property real ringInsetRatio: 0.025
    }

    // --- Q1 ---
    // Queue panel docked at the right of the library (it narrows the page, never covers the
    // player bar) and its row editing: drag handle, insertion line, the row being dragged.
    readonly property QtObject queue: QtObject {
        readonly property int width: theme.panel.queueWidth
        readonly property int inset: theme.space.sm                         // gap to the window edge and to the player bar
        readonly property int handleWidth: theme.icon.sm + theme.space.sm    // drag handle column
        readonly property real dropLine: theme.stroke.focus                 // insertion indicator thickness
        readonly property int dropLineInset: theme.space.lg                 // indicator starts/ends inside the row edges
        readonly property color dropColor: theme.accent
        readonly property color dragFill: theme.accentSoft                   // the row being moved
    }

    // --- G4 --- Drag handles on the edges of the sidebar and the queue panel (resize).
    readonly property QtObject resize: QtObject {
        readonly property int handle: theme.px(6)                    // grab strip, centered on the edge
        readonly property int line: theme.stroke.focus               // indicator shown on hover / drag
        readonly property int lineInset: theme.space.lg              // indicator starts/ends inside the edge
        readonly property color lineColor: theme.controlBorder       // >= 3:1 on the panels beside it
        readonly property color lineActive: theme.accent             // while dragging
        readonly property int minPage: theme.px(360)                 // page width the two panels always leave
        readonly property int keyStep: theme.space.lg                // ←/→ on a focused handle
    }

    // --- F3F4 ---
    // Columns of the song lists (# | Título | Álbum | Fecha en que se agregó | Duración), their
    // header strip, and the in-list search / sort toolbar. Columns hide in order (Fecha, then
    // Álbum) below these row widths; the docked queue narrows the page past both.
    readonly property QtObject trackList: QtObject {
        // U4: like Spotify, a row's hover surface starts at the page edge (flush with the header
        // art) and its content ("#", thumbnail) sits this far inside it; the "#" column is as
        // wide as the digits of the longest index of that list, not a fixed 5 digits.
        readonly property int rowInset: theme.space.lg
        readonly property int rowArt: theme.size.trackRow - 2 * theme.space.sm      // row thumbnail
        // Right-hand columns every row has: heart, duration, "..." (their widths, no spacing).
        readonly property int trailingWidth: theme.icon.sm + durationWidth + theme.size.iconButton
        readonly property int albumMinRowWidth: theme.px(560)
        readonly property int addedMinRowWidth: theme.px(760)
        readonly property real albumShare: 0.34                         // of the width left after the fixed columns
        readonly property int addedWidth: theme.size.coverMin - theme.space.xxxl     // fits "Fecha en que se agregó" at type.caption
        readonly property int durationWidth: theme.space.xxxl
        readonly property int headerRow: theme.size.iconButton          // column header strip height
        readonly property int compactRow: theme.size.trackRow - theme.space.lg      // "Ver como: Compacta"
        readonly property real compactTitleShare: 0.6                    // compact rows: title's max share of its column
        readonly property int searchFieldWidth: theme.size.sidebar      // "Buscar en la playlist"
        readonly property int menuWidth: theme.size.sidebar             // "Clasificar por" / "Ver como" menu
        readonly property int toolbarRoomyWidth: theme.size.sidebar * 3 - theme.space.xxxl   // sort button keeps its label above this
    }

    // --- F6 --- Artist page (popular list, discography chips, about card).
    readonly property QtObject artistPage: QtObject {
        readonly property int popularCollapsed: 5       // "Populares" shows 5, "Ver más" up to 10
        readonly property int bioLines: 4               // "Acerca de" before "Leer más"
        readonly property int aboutPhoto: theme.library.headerCover   // portrait in "Acerca de"
        readonly property int chipHeight: theme.library.chipHeight // obsoleto: use library.chipHeight
    }

    // --- F1F2 --- Search: suggestions popup, best result card, songs beside it.
    readonly property QtObject search: QtObject {
        readonly property int fieldWidth: theme.size.sidebar * 2 + theme.space.xxxl   // obsoleto (T3): el campo es searchBox
        readonly property int bestCardWidth: theme.size.sidebar + theme.space.xxxl * 2
        // U4: "Mejor resultado" like Spotify: as tall as the 4 song rows beside it, uniform
        // padding, art (round for artists) with the title and type under it, a play disc that
        // rises in at the bottom right on hover/focus.
        readonly property real bestShare: 0.4           // of the row when side by side (Spotify 2fr / 3fr)
        readonly property int bestArt: theme.size.trackRow * 2 - theme.space.lg   // 96
        readonly property int bestPadding: theme.space.lg + theme.space.xs        // 20
        readonly property int bestPlay: theme.size.titleBar + theme.space.xs      // 48
        readonly property int songsShown: 4            // "Canciones" next to "Mejor resultado"
        readonly property int sideBySideWidth: theme.size.sidebar * 3   // below: best result stacks above songs
    }

    // --- U4 --- Profile page.
    readonly property QtObject profile: QtObject {
        readonly property int topTracksShown: 4         // "Canciones más escuchadas este mes", as Spotify
    }

    // --- U5 --- F5 genre/mood chips over "Tus me gusta" and playlists. Durations come from
    // `motion` (Modo rendimiento / Reducir movimiento make them instant); these are distances.
    readonly property QtObject tagChips: QtObject {
        readonly property int rise: theme.space.xs                 // each chip rises 4 px as it fades in
        readonly property int stagger: 20 * theme.motion.scale    // ms between chips when the tags arrive
        readonly property int staggerMax: 6                        // chips past this enter without delay
    }

    // --- S9/N1 --- Visualizer drawn behind the Library player bar.
    readonly property QtObject playerViz: QtObject {
        readonly property real opacity: 0.24
    }

    // --- H1 --- Inicio: quick access tiles, top tracks.
    readonly property QtObject home: QtObject {
        readonly property int quickTile: theme.size.trackRow            // tile height = its art
        readonly property int quickWideWidth: theme.size.sidebar * 3    // 4 columns above, 2 below
        readonly property int topTracksShown: 5
        readonly property int quickCount: 8                             // "Acceso rápido" tiles shown
        readonly property color likedFrom: "#450AF5"                    // Liked Songs tile gradient (Spotify's own)
        readonly property color likedTo: "#8C7CF0"
        readonly property color likedInk: "#FFFFFF"
    }

    // --- S1-2 --- Entrances of dialogs, panels, menus, pages and Inicio. Durations and curves
    // come from `motion` (so Modo rendimiento / Reducir movimiento turn them off); these are
    // only the distances and scales. Never scale from 0.
    readonly property QtObject entrance: QtObject {
        readonly property real dialogScale: 0.97      // dialogs, sheets, pickers start here
        readonly property real dialogExitScale: 0.98  // and leave toward this
        readonly property real menuScale: 0.98        // menus, suggestions, small popovers
        readonly property int pageShift: theme.space.sm      // library page rises 8 px
        readonly property int cardShift: theme.space.md      // Inicio cards rise 12 px
        readonly property int popoverShift: theme.space.sm   // volume panel rises 8 px
        readonly property real queueShift: 0.15        // queue slides in from 15% of its width
        readonly property int stagger: 40 * theme.motion.scale   // ms between Inicio cards
        readonly property int staggerMax: 8            // cards past this enter without delay
    }

    // --- S1-3 ---
    // Amplitudes of the micro-motions (durations and curves stay in `motion`). Scales are
    // transforms only; nothing here moves layout.
    readonly property QtObject feel: QtObject {
        readonly property real pressScale: 0.96        // compact buttons settle while held
        readonly property real pressScaleWide: 0.98    // full-width rows/buttons: the same feel, fewer px
        readonly property real hoverLift: 1.03         // play disc under a mouse pointer
        readonly property real knobGrow: 1.25          // slider knob while dragged
        readonly property real likePop: 1.2           // heart when a song is saved
        readonly property real dotFrom: 0.5           // shuffle dot grows in from half size (never from 0)
        readonly property int lyricJumpLines: 3        // a longer lyric jump (seek) is instant
        readonly property int lyricsShift: theme.space.xl   // lyrics column slides in by this much
        readonly property real waveSpeed: 20 * theme.zoomFactor           // progress wave drift, px per second
        readonly property real waveMaxStep: 0.1        // s: longest gap between clock ticks it bridges
        // "Sonando ahora" bars in the playing row: visualizer bands they follow (low, mid, high),
        // their heights at rest (paused or visualizer off), and the lowest they go.
        readonly property var nowPlayingBands: [4, 14, 28]
        readonly property var nowPlayingRest: [0.55, 0.9, 0.4]
        readonly property real nowPlayingFloor: 0.2
    }

    // --- U1 (S9/N2) --- Ajustes as a page inside the library (components/library/SettingsPage.qml),
    // like Spotifast: a left-aligned reading column of section cards, each row text left /
    // control right. Distances only; durations and curves come from `motion` (Modo rendimiento
    // and Reducir movimiento turn the movement off).
    readonly property QtObject settingsPage: QtObject {
        readonly property int column: theme.px(800)                 // reading column (left aligned)
        readonly property int cardRadius: theme.radius.lg + theme.space.xs   // 18: large, soft cards
        readonly property int cardPadding: theme.space.xl - theme.space.xs   // 20 around the rows
        readonly property int rowMinHeight: theme.size.titleBar + theme.space.xs   // 48
        readonly property real controlShare: 0.6                    // wider controls drop under their text
        readonly property int subIndent: theme.space.xl              // G4: a sub-option under its master switch
        readonly property int searchWidth: theme.px(360)            // «Buscar en los ajustes»
        readonly property int fieldWidth: theme.px(240)             // device name, proxy host…
        readonly property int portWidth: theme.px(96)
        readonly property int sectionGap: theme.space.xxl           // between sections
        readonly property int logo: theme.px(64)                    // Acerca de
        readonly property color cardFill: theme.surface1
        readonly property color cardBorder: theme.hairline
        readonly property color warnFill: theme.accentSoft          // «Se aplicará al reconectar»
        // Equalizer: response curve over 11 vertical sliders (Pre + 10 bands).
        readonly property int eqCurveHeight: theme.px(96)
        readonly property int eqSliderHeight: theme.px(150)
        readonly property int eqColumnMin: theme.px(44)
        readonly property real eqRangeDb: 12                        // ±12 dB
        readonly property real eqStepDb: 0.5
        readonly property int eqCurvePoints: 96                     // samples of the drawn curve
        readonly property real eqCurveWidthOct: 0.75                // band bell width (octaves) in the drawing
        readonly property real eqFillAlpha: 0.18
        readonly property int presetPill: theme.size.iconButton - theme.space.xs   // 28
        // Motion: sections rise in on entry (entrance.cardShift, entrance.stagger); rows that come
        // back while filtering only fade (motion.instant); preset pills settle on press.
    }

    // --- E5b --- DJ Livi: player bar button, its tooltip and the Inicio card.
    // The "Livi is talking" breath is a halo behind the DJ glyph: scale + opacity only, one half
    // (grow, then shrink) per `pulse`. Bound to motion.scale, so Modo rendimiento / Reducir
    // movimiento leave a still halo instead.
    readonly property QtObject dj: QtObject {
        readonly property int pulse: 900 * theme.motion.scale   // ms per half breath (period 1.8 s)
        readonly property real pulseScale: 1.22         // halo grows to this (transform, no relayout)
        readonly property real haloRest: 0.16           // halo opacity between breaths / when still
        readonly property real haloPeak: 0.34           // halo opacity at the top of a breath
        readonly property real glyphBreath: 1.08        // card glyph swells this much per breath
        readonly property int tipDelay: theme.tip.delay   // hover delay before the tooltip (not motion)
        readonly property int tipWidth: theme.size.sidebar + theme.space.xxl   // tooltip text measure
        readonly property int cardGlyph: theme.size.playDisc + theme.space.md  // glyph on the card art
    }

    // --- T2 --- Sleep-timer indicator in the title bar (quiet, secondary). T8: its tone is the
    // view's secondary ink (TitleBar.inkSecondary), not an opacity: contrast stays measured.
    readonly property QtObject titleTimer: QtObject {
        readonly property int icon: theme.icon.sm - theme.space.xs   // glyph next to the text
        readonly property int gap: theme.space.xs         // glyph to text
    }

    // --- T4 --- Clicks that switch the visualizer (library player bar free space, Now Playing
    // background and cover). No motion of their own: the visualizer's own entrance plays.
    readonly property QtObject nowPlayingClicks: QtObject {
        // A click on the player bar's free space only counts this far from any control, so
        // missing a button by a little never switches the visualizer. Never under 8 px, even
        // at 75 % zoom (T4b): a near miss is about the pointer, not the zoom.
        readonly property int barMargin: Math.max(8, theme.space.sm)
        // Keyboard focus ring around the clickable cover (outside the art).
        readonly property int focusGap: theme.space.xs
    }
    // --- T3 --- Search box in the library's top bar (like Spotify: a pill centered in the title
    // strip, the suggestions drop under it over the page). Distances only; the popup entrance
    // uses `entrance.menuScale` and `motion.instant`.
    readonly property QtObject searchBox: QtObject {
        // 36, inside the 44 px strip; T8: never under the 32 px hit target (zoom 75 %).
        readonly property int height: Math.max(theme.size.iconButton + theme.space.xs, theme.size.minimumHitTarget)
        readonly property int maxWidth: theme.px(480)              // Spotify's field measure
        readonly property int minWidth: theme.px(200)              // below: the «Biblioteca» crumb steps aside
        readonly property int gap: theme.space.lg                  // clearance to the buttons around it
        readonly property int popupMinWidth: theme.px(360)         // a narrow field still drops a readable list
        readonly property int popupMargin: theme.space.md          // the popup stays this far inside the window
        readonly property int hintMinWidth: theme.px(300)          // the shortcut chip shows from this width
        readonly property int hintPadding: theme.space.xs          // inside the shortcut chip
        readonly property int iconInset: theme.space.md            // search glyph from the left edge
        readonly property color fill: theme.surface2
        readonly property color fillHover: theme.surface3
    }
}
