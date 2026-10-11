import QtQuick
import QtTest
import "../../theme"

// Lavado de cara, regla 1: movimiento de interaccion (finito) vs ambiente (bucles).
// Interaccion: Theme.motion.* depende solo de ventana visible, foco/preferencia de fondo,
// Modo rendimiento y Reducir movimiento; NO de la musica. Ambiente: effectsAllowed/driftActive
// ademas exigen musica sonando.
Item {
    width: 100; height: 100

    TestCase {
        name: "Feel1"
        when: windowShown

        function init() {
            Theme.performanceMode = false; Theme.reduceMotion = false; Theme.pauseInBackground = false
            Theme.windowVisible = true; Theme.windowFocused = true; Theme.playing = true; Theme.drift = true
        }
        function cleanup() { init() }

        function test_pause_keeps_interaction_and_stops_ambient() {
            Theme.playing = false
            verify(Theme.interactionAllowed)
            compare(Theme.motion.scale, 1)
            verify(Theme.motion.instant > 0 && Theme.motion.fast > 0 && Theme.motion.base > 0 && Theme.motion.slow > 0)
            verify(Theme.motion.viewFade > 0 && Theme.motion.themeFade > 0 && Theme.motion.zoomFade > 0)
            verify(Theme.entrance.stagger > 0)
            verify(!Theme.effectsAllowed)
            verify(!Theme.driftActive)
        }
        function test_playing_enables_both() {
            verify(Theme.interactionAllowed); verify(Theme.effectsAllowed); verify(Theme.driftActive)
        }
        function test_minimized_stops_both() {
            Theme.windowVisible = false
            verify(!Theme.interactionAllowed); verify(!Theme.effectsAllowed)
            compare(Theme.motion.scale, 0); compare(Theme.motion.fast, 0); compare(Theme.motion.viewFade, 0)
        }
        function test_performance_mode_zeroes_everything() {
            Theme.performanceMode = true
            compare(Theme.motion.scale, 0); compare(Theme.motion.instant, 0); compare(Theme.motion.slow, 0)
            compare(Theme.motion.viewFade, 0); compare(Theme.motion.coverFade, 0); compare(Theme.motion.zoomFade, 0)
            verify(!Theme.driftActive)
        }
        function test_reduce_motion_stops_movement_keeps_fades() {
            Theme.reduceMotion = true
            compare(Theme.motion.scale, 0); compare(Theme.motion.base, 0)
            verify(Theme.motion.viewFade > 0)
        }
        function test_background_preference_applies_to_interaction() {
            Theme.windowFocused = false
            verify(Theme.motion.fast > 0)
            Theme.pauseInBackground = true
            compare(Theme.motion.fast, 0); verify(!Theme.effectsAllowed)
        }
    }
}
