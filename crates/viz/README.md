# velo-viz

`VizEngine::new()` implements `velo_domain::Visualizer`. The caller supplies
`enabled && playing && visible && focused` and the Spotify PID to `set_active`.
The same state/PID is idempotent; a new PID cancels the previous capture first.

Windows capture requires build 20348 or later. The worker verifies the executable
name and retains its process handle, then activates **only**
`VAD\Process_Loopback` with `INCLUDE_TARGET_PROCESS_TREE`. There is no device-wide
loopback fallback. The stream is shared, event-driven stereo at 48 kHz, float32
with a fresh PCM16 activation if float initialization fails. Activation errors,
unsupported Windows, missing/invalid PIDs and stream failure select simulation.
`is_simulated()` reports the current mode after asynchronous initialization.
With Spotify absent, `set_active(true, None)` animates simulated bands; normally
the bridge passes `false` because there is no playing session, so it stays idle.

The worker downmixes into a 2048-sample circular buffer and reuses the FFT plan,
scratch buffer, Hann window and logarithmic bin map. The real signal's positive
FFT half becomes 48 peak-magnitude bands (40 Hz–16 kHz), normalized from −70 dB,
scaled by sensitivity, clipped and smoothed. Publication follows the display refresh rate
(30–240 Hz, 60 Hz when the screen rate is unavailable);
unchanged bands do not notify watchers. The immediate zero on deactivation is a
lifecycle notification, independent of the animation frame limit.

Deactivation gates publication and clears bands under the same lock, so a stale
worker frame cannot follow the zero. It wakes the capture event or simulated
condition-variable wait. WASAPI is stopped and released on its owning MTA thread;
the idle worker then waits on a condition variable **without a timeout**. There
are no idle timers, FFT calls, capture streams or recurring wakes. Active capture
waits have a 100 ms safety timeout; pending activation checks cancellation every
20 ms. Drop waits at most 150 ms for completion. If an OS/driver call stalls, the
worker retains ownership and cleans up when that call returns instead of holding
the UI indefinitely.

Run offline checks:

```powershell
$env:CARGO_NET_OFFLINE = 'true'
cargo fmt -p velo-viz
cargo clippy -p velo-viz --tests --examples -- -D warnings
cargo test -p velo-viz
cargo build -p velo-viz --examples
cargo run -p velo-viz --example viz_probe --release
# Or target an explicit Spotify process:
cargo run -p velo-viz --example viz_probe --release -- --pid 1234
```

The probe finds a Spotify.exe whose parent is not Spotify.exe, runs for eight
seconds, prints 48 text levels four times per second and labels every line REAL
or SIMULADO. CPU is the probe process's kernel+user CPU delta from GetProcessTimes
divided by elapsed wall time (100% means one fully occupied logical CPU). It
includes startup, console output and teardown; it is not Spotify's CPU usage.
