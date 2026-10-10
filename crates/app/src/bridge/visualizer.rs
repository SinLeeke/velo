//! `Visualizer` singleton. Settings persist through `engine::prefs`; capture
//! and FFT live in velo-viz on their own thread and only run while `running` (see
//! `engine::viz`). `bands` is pushed at most at the display rate (30..=240 Hz, 60 by default), and only changes while running.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_f64 = cxx_qt_lib::QList<f64>;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, enabled)]
        #[qproperty(QString, style)]
        #[qproperty(QString, shape)]
        #[qproperty(f64, sensitivity)]
        #[qproperty(bool, running, READ, NOTIFY)]
        #[qproperty(bool, simulated, READ, NOTIFY)]
        #[qproperty(QList_f64, bands, READ, NOTIFY)]
        type Visualizer = super::VisualizerRust;

        #[qinvokable]
        fn set_surfaces(self: &Visualizer, now_playing_visible: bool, library_bar_visible: bool);
        #[qinvokable]
        fn set_window_state(self: &Visualizer, visible: bool, focused: bool);
        /// The display's real refresh rate in Hz (clamped to 30..=240; 60 until informed).
        #[qinvokable]
        fn set_display_rate(self: Pin<&mut Visualizer>, hz: f64);
    }

    impl cxx_qt::Threading for Visualizer {}
    impl cxx_qt::Initialize for Visualizer {}
}

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString};
use velo_domain::{VIZ_BANDS, Visualizer as _, VizShape, VizStyle};

use crate::engine::clock::running;
use crate::engine::core;

pub const SENSITIVITY_MIN: f64 = 0.25;
pub const SENSITIVITY_MAX: f64 = 2.0;

pub struct VisualizerRust {
    enabled: bool,
    style: QString,
    shape: QString,
    sensitivity: f64,
    running: bool,
    simulated: bool,
    bands: QList<f64>,
    frame_gate: crate::engine::viz::FrameGate,
}

pub fn style_name(s: VizStyle) -> &'static str {
    match s {
        VizStyle::Normal => "normal",
        VizStyle::Circular => "circular",
    }
}

pub fn parse_style(s: &str) -> VizStyle {
    if s == "circular" {
        VizStyle::Circular
    } else {
        VizStyle::Normal
    }
}

pub fn shape_name(s: VizShape) -> &'static str {
    match s {
        VizShape::Bars => "bars",
        VizShape::Wave => "wave",
    }
}

pub fn parse_shape(s: &str) -> VizShape {
    if s == "bars" {
        VizShape::Bars
    } else {
        VizShape::Wave
    }
}

pub fn clamp_sensitivity(v: f64) -> f64 {
    if v.is_nan() {
        1.0
    } else {
        v.clamp(SENSITIVITY_MIN, SENSITIVITY_MAX)
    }
}

impl Default for VisualizerRust {
    fn default() -> Self {
        let c = core();
        let v = c.env.prefs.settings().visualizer;
        Self {
            enabled: v.enabled,
            style: QString::from(style_name(v.style)),
            shape: QString::from(shape_name(v.shape)),
            sensitivity: f64::from(v.sensitivity),
            running: running(&c.env.flags.borrow()),
            simulated: c.env.viz.is_simulated(),
            bands: std::iter::repeat_n(0.0, VIZ_BANDS).collect(),
            frame_gate: crate::engine::viz::FrameGate::default(),
        }
    }
}

fn persist(q: &ffi::Visualizer) {
    let (enabled, style, shape) = (*q.enabled(), q.style().to_string(), q.shape().to_string());
    let sensitivity = clamp_sensitivity(*q.sensitivity()) as f32;
    core().env.prefs.update_settings(|s| {
        s.visualizer.enabled = enabled;
        s.visualizer.style = parse_style(&style);
        s.visualizer.shape = parse_shape(&shape);
        s.visualizer.sensitivity = sensitivity;
    });
}

impl cxx_qt::Initialize for ffi::Visualizer {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().on_enabled_changed(|q| persist(&q)).release();
        self.as_mut().on_style_changed(|q| persist(&q)).release();
        self.as_mut().on_shape_changed(|q| persist(&q)).release();
        self.as_mut()
            .on_sensitivity_changed(|q| persist(&q))
            .release();

        let env = core().env.clone();

        // `running` follows the gate flags (event-driven).
        let thread = self.qt_thread();
        let mut flags = env.flags.subscribe();
        env.rt.spawn(async move {
            loop {
                let now = running(&flags.borrow_and_update());
                if thread.queue(move |q| q.set_running_value(now)).is_err() {
                    break;
                }
                if flags.changed().await.is_err() {
                    break;
                }
            }
        });

        // Bands: the engine follows the display refresh rate and publishes nothing while inactive.
        let thread = self.qt_thread();
        let engine = env.viz.clone();
        let mut bands = engine.bands();
        env.rt.spawn(async move {
            while bands.changed().await.is_ok() {
                let frame = *bands.borrow_and_update();
                let simulated = engine.is_simulated();
                let queued = thread.queue(move |q| q.set_frame(&frame, simulated));
                if queued.is_err() {
                    break;
                }
            }
        });
    }
}

impl ffi::Visualizer {
    fn set_running_value(mut self: Pin<&mut Self>, value: bool) {
        if self.running != value {
            self.as_mut().rust_mut().running = value;
            self.as_mut().running_changed();
        }
    }

    fn set_frame(mut self: Pin<&mut Self>, frame: &[f32; VIZ_BANDS], simulated: bool) {
        let flags = *core().env.flags.borrow();
        if !self
            .as_mut()
            .rust_mut()
            .frame_gate
            .accept(&flags, std::time::Instant::now())
        {
            return;
        }
        if self.simulated != simulated {
            self.as_mut().rust_mut().simulated = simulated;
            self.as_mut().simulated_changed();
        }
        let bands: QList<f64> = frame.iter().map(|v| f64::from(*v)).collect();
        if self.bands != bands {
            self.as_mut().rust_mut().bands = bands;
            self.as_mut().bands_changed();
        }
    }

    pub fn set_display_rate(mut self: Pin<&mut Self>, hz: f64) {
        core().env.viz.set_target_fps(hz);
        self.as_mut().rust_mut().frame_gate.set_hz(hz);
    }

    pub fn set_surfaces(&self, now_playing_visible: bool, library_bar_visible: bool) {
        core().env.update_flags(|f| {
            f.now_playing_visible = now_playing_visible;
            f.library_bar_visible = library_bar_visible;
        });
    }

    pub fn set_window_state(&self, visible: bool, focused: bool) {
        core().env.update_flags(|f| {
            f.visible = visible;
            f.focused = focused;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_and_shape_round_trip() {
        for s in [VizStyle::Normal, VizStyle::Circular] {
            assert_eq!(parse_style(style_name(s)), s);
        }
        for s in [VizShape::Bars, VizShape::Wave] {
            assert_eq!(parse_shape(shape_name(s)), s);
        }
        assert_eq!(parse_style("junk"), VizStyle::Normal);
        assert_eq!(parse_shape("junk"), VizShape::Wave);
    }

    #[test]
    fn sensitivity_is_clamped_to_the_contract_range() {
        assert_eq!(clamp_sensitivity(0.0), SENSITIVITY_MIN);
        assert_eq!(clamp_sensitivity(9.0), SENSITIVITY_MAX);
        assert_eq!(clamp_sensitivity(1.2), 1.2);
        assert_eq!(clamp_sensitivity(f64::NAN), 1.0);
    }
}
