import QtQuick
import QtTest
import "../../theme"
import "../../components/controls"

// Regression: with every motion duration at 0 (music paused, Modo rendimiento, Reducir
// movimiento) an OpacityAnimator inside a Behavior never applies its end value, so a switch
// slid its knob but never turned green. Every OpacityAnimator runs at least 1 ms.
Item {
    width: 200; height: 100

    VToggle { id: toggle; text: "t" }

    TestCase {
        name: "FadeZero"
        when: windowShown

        function cleanup() { Theme.performanceMode = false }

        function test_toggle_turns_on_with_zero_durations() {
            Theme.performanceMode = true
            compare(Theme.motion.fast, 0)
            toggle.toggle()
            verify(toggle.checked)
            const onTrack = toggle.background.children[1]
            tryCompare(onTrack, "opacity", 1, 500)
            toggle.toggle()
            tryCompare(onTrack, "opacity", 0, 500)
        }
    }
}
