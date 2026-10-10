//! Native window chrome.
//!
//! The Win32 side lives in `cpp/window_chrome.cpp` (a `QAbstractNativeEventFilter`): it keeps the
//! system frame styles so DWM gives the window its shadow, rounded corners, Snap, the minimize
//! and maximize animations and the system menu, while Qt Quick draws the whole client area.
//! The decision of what a point of the window is (resize edge, caption, client) is pure and
//! lives here in [`hit_test`], so it can be unit tested; the C++ filter calls it through
//! [`velo_hit_test`] for every `WM_NCHITTEST`.

use std::ffi::c_void;

/// Rectángulo físico Win32, también usado por WM_NCCALCSIZE.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Insets del marco que sobresale del área de trabajo; sin escalado ni redondeo Qt.
/// En normal/snap no se recorta: Windows ya eligió el rectángulo de la ventana.
pub fn client_insets(window: PhysicalRect, work: PhysicalRect, maximized: bool) -> PhysicalRect {
    if !maximized {
        return PhysicalRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
    }
    PhysicalRect {
        left: work.left.saturating_sub(window.left).max(0),
        top: work.top.saturating_sub(window.top).max(0),
        right: window.right.saturating_sub(work.right).max(0),
        bottom: window.bottom.saturating_sub(work.bottom).max(0),
    }
}

/// # Safety
/// Ambos punteros deben señalar rectángulos inicializados que vivan durante la llamada.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velo_client_insets(
    window: *const PhysicalRect,
    work: *const PhysicalRect,
    maximized: bool,
) -> PhysicalRect {
    // SAFETY: el filtro nativo pasa dos valores locales válidos.
    client_insets(unsafe { *window }, unsafe { *work }, maximized)
}

/// `WM_NCHITTEST` results, with the Win32 values (`HTCLIENT`, `HTCAPTION`, ...).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Client = 1,
    Caption = 2,
    Left = 10,
    Right = 11,
    Top = 12,
    TopLeft = 13,
    TopRight = 14,
    Bottom = 15,
    BottomLeft = 16,
    BottomRight = 17,
}

/// One hit-test query. All values are physical pixels relative to the window rectangle's
/// top-left corner (the rectangle from `GetWindowRect`, which may include invisible frame
/// borders outside the client area).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HitTestInput {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Client rectangle inside the window rectangle.
    pub client_left: i32,
    pub client_top: i32,
    pub client_right: i32,
    pub client_bottom: i32,
    /// Resize band measured from the client edge inwards (already DPI scaled).
    pub border: i32,
    /// Height of the draggable title bar strip, from the client top (already DPI scaled).
    pub caption: i32,
    pub maximized: bool,
    /// The point lies over a QML item that takes clicks (window buttons, controls).
    pub over_interactive: bool,
}

/// Classifies a point of the window.
///
/// Interactive items in the client title bar win even over resize corners, so their whole
/// click area works. Unoccupied edges resize unless maximized; the remaining title bar
/// strip drags the window; the rest is client area.
pub fn hit_test(i: &HitTestInput) -> Hit {
    let outside = i.x < 0 || i.y < 0 || i.x >= i.width || i.y >= i.height;
    let in_caption = i.y >= i.client_top && i.y < i.client_top + i.caption;
    let in_client = i.x >= i.client_left
        && i.x < i.client_right
        && i.y >= i.client_top
        && i.y < i.client_bottom;
    if !outside && in_client && in_caption && i.over_interactive {
        return Hit::Client;
    }
    if !i.maximized && !outside {
        // Resize bands: the invisible frame outside the client area plus `border` inside it.
        let left = i.x < i.client_left + i.border;
        let right = i.x >= i.client_right - i.border;
        let top = i.y < i.client_top + i.border;
        let bottom = i.y >= i.client_bottom - i.border;
        // Corners get a wider grab area along each edge, as in native windows.
        let corner = 2 * i.border;
        let near_left = i.x < i.client_left + corner;
        let near_right = i.x >= i.client_right - corner;
        let near_top = i.y < i.client_top + corner;
        let near_bottom = i.y >= i.client_bottom - corner;
        match () {
            _ if (top && near_left) || (left && near_top) => return Hit::TopLeft,
            _ if (top && near_right) || (right && near_top) => return Hit::TopRight,
            _ if (bottom && near_left) || (left && near_bottom) => return Hit::BottomLeft,
            _ if (bottom && near_right) || (right && near_bottom) => return Hit::BottomRight,
            _ if top => return Hit::Top,
            _ if bottom => return Hit::Bottom,
            _ if left => return Hit::Left,
            _ if right => return Hit::Right,
            _ => {}
        }
    }
    if in_caption && !i.over_interactive {
        Hit::Caption
    } else {
        Hit::Client
    }
}

/// Called from `cpp/window_chrome.cpp` for every `WM_NCHITTEST`.
///
/// # Safety
/// `input` must point to a valid, initialised [`HitTestInput`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velo_hit_test(input: *const HitTestInput) -> u32 {
    // SAFETY: the caller passes a pointer to a stack value that outlives this call.
    let input = unsafe { &*input };
    hit_test(input) as u32
}

unsafe extern "C" {
    fn velo_window_prepare(mica: bool);
    fn velo_window_chrome_install(engine: *mut c_void, mica: bool) -> bool;
}

/// Must run after `QGuiApplication` exists and before any QML window is created.
/// Keeps Qt Quick on the GPU (Direct3D 11 on Windows) and, with Mica, asks for an alpha buffer.
pub fn prepare(mica: bool) {
    // SAFETY: plain C function without preconditions beyond a live QGuiApplication.
    unsafe { velo_window_prepare(mica) }
}

/// Attaches the native chrome to the engine's root `Window`. Returns false if there is no
/// root window (e.g. Main.qml failed to load).
pub fn install(engine: &mut cxx::UniquePtr<cxx_qt_lib::QQmlApplicationEngine>, mica: bool) -> bool {
    // SAFETY: the pointer comes from a live UniquePtr and is only used during the call.
    unsafe { velo_window_chrome_install(engine.as_mut_ptr().cast::<c_void>(), mica) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximized_client_fills_work_area_at_each_dpi_and_taskbar_edge() {
        // Monitores a izquierda/arriba del principal y barras en las cuatro posiciones.
        for work in [
            PhysicalRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            PhysicalRect {
                left: -2520,
                top: -1440,
                right: 0,
                bottom: 0,
            },
            PhysicalRect {
                left: -2560,
                top: -1400,
                right: 0,
                bottom: 0,
            },
            PhysicalRect {
                left: 1920,
                top: 0,
                right: 4440,
                bottom: 1440,
            },
        ] {
            for frame in [8, 10, 12, 16] {
                let window = PhysicalRect {
                    left: work.left - frame,
                    top: work.top - frame,
                    right: work.right + frame,
                    bottom: work.bottom + frame,
                };
                let inset = client_insets(window, work, true);
                assert_eq!(
                    inset,
                    PhysicalRect {
                        left: frame,
                        top: frame,
                        right: frame,
                        bottom: frame
                    }
                );
                assert_eq!(
                    PhysicalRect {
                        left: window.left + inset.left,
                        top: window.top + inset.top,
                        right: window.right - inset.right,
                        bottom: window.bottom - inset.bottom,
                    },
                    work
                );
            }
            assert_eq!(
                client_insets(work, work, true),
                PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0
                }
            );
        }
    }

    #[test]
    fn normal_and_snapped_windows_keep_their_full_client() {
        let work = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let snapped = PhysicalRect {
            left: -8,
            top: -8,
            right: 960,
            bottom: 1048,
        };
        assert_eq!(
            client_insets(snapped, work, false),
            PhysicalRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0
            }
        );
        let asymmetric = PhysicalRect {
            left: -12,
            top: 0,
            right: 1928,
            bottom: 1056,
        };
        assert_eq!(
            client_insets(asymmetric, work, true),
            PhysicalRect {
                left: 12,
                top: 0,
                right: 8,
                bottom: 16
            }
        );
    }

    /// A 1100×680 client at 150 % scale inside a window rect that also has an invisible
    /// 8 px frame on the left, right and bottom (general case; Velo's own window has no
    /// frame, see `frameless_client_uses_inner_border`).
    fn input(x: i32, y: i32) -> HitTestInput {
        let (w, h) = (1650 + 16, 1020 + 8);
        HitTestInput {
            x,
            y,
            width: w,
            height: h,
            client_left: 8,
            client_top: 0,
            client_right: w - 8,
            client_bottom: h - 8,
            border: 9,
            caption: 66,
            maximized: false,
            over_interactive: false,
        }
    }

    #[test]
    fn invisible_frame_resizes() {
        assert_eq!(hit_test(&input(2, 500)), Hit::Left);
        assert_eq!(hit_test(&input(1660, 500)), Hit::Right);
        assert_eq!(hit_test(&input(800, 1025)), Hit::Bottom);
        assert_eq!(hit_test(&input(2, 1025)), Hit::BottomLeft);
        assert_eq!(hit_test(&input(1663, 1025)), Hit::BottomRight);
    }

    #[test]
    fn top_band_inside_client_resizes() {
        assert_eq!(hit_test(&input(800, 0)), Hit::Top);
        assert_eq!(hit_test(&input(800, 8)), Hit::Top);
        assert_eq!(hit_test(&input(800, 9)), Hit::Caption);
        assert_eq!(hit_test(&input(10, 3)), Hit::TopLeft);
        assert_eq!(hit_test(&input(3, 15)), Hit::TopLeft);
        assert_eq!(hit_test(&input(1650, 2)), Hit::TopRight);
    }

    #[test]
    fn title_bar_drags_except_over_buttons() {
        assert_eq!(hit_test(&input(800, 30)), Hit::Caption);
        assert_eq!(hit_test(&input(800, 65)), Hit::Caption);
        assert_eq!(hit_test(&input(800, 66)), Hit::Client);
        let button = HitTestInput {
            over_interactive: true,
            ..input(40, 20)
        };
        assert_eq!(hit_test(&button), Hit::Client);
    }

    #[test]
    fn title_bar_buttons_win_over_resize_bands_and_corners() {
        // All of the six logical pixels at 150 % DPI, including occupied corners.
        for y in 0..9 {
            for x in [8, 10, 40, 800, 1650, 1657] {
                let button = HitTestInput {
                    over_interactive: true,
                    ..input(x, y)
                };
                assert_eq!(hit_test(&button), Hit::Client, "button at ({x}, {y})");
                assert_eq!(
                    hit_test(&HitTestInput {
                        maximized: true,
                        ..button
                    }),
                    Hit::Client
                );
            }
        }
        // An interactive flag cannot consume the invisible frame outside the client.
        let frame = HitTestInput {
            over_interactive: true,
            ..input(2, 3)
        };
        assert_eq!(hit_test(&frame), Hit::TopLeft);
        assert_eq!(hit_test(&input(10, 3)), Hit::TopLeft);
        assert_eq!(hit_test(&input(1650, 3)), Hit::TopRight);
    }

    #[test]
    fn maximized_has_no_resize_edges() {
        let max = |x, y| HitTestInput {
            maximized: true,
            ..input(x, y)
        };
        assert_eq!(hit_test(&max(800, 0)), Hit::Caption);
        assert_eq!(hit_test(&max(8, 500)), Hit::Client);
        assert_eq!(hit_test(&max(1657, 1019)), Hit::Client);
    }

    #[test]
    fn frameless_client_uses_inner_border() {
        // Client covers the whole window (no invisible frame).
        let i = |x, y| HitTestInput {
            client_left: 0,
            client_right: 1100,
            client_bottom: 680,
            width: 1100,
            height: 680,
            border: 6,
            caption: 44,
            ..input(x, y)
        };
        assert_eq!(hit_test(&i(3, 300)), Hit::Left);
        assert_eq!(hit_test(&i(1097, 300)), Hit::Right);
        assert_eq!(hit_test(&i(500, 677)), Hit::Bottom);
        assert_eq!(hit_test(&i(500, 20)), Hit::Caption);
        assert_eq!(hit_test(&i(500, 300)), Hit::Client);
    }

    #[test]
    fn ffi_matches_win32_values() {
        let i = input(800, 30);
        // SAFETY: pointer to a live local.
        assert_eq!(unsafe { velo_hit_test(&i) }, 2);
        assert_eq!(Hit::BottomRight as u32, 17);
    }
}
