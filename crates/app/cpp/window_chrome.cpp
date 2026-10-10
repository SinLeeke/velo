// Velo native window chrome (Windows 11).
//
// Qt 6.10's own option (Qt::ExpandedClientAreaHint) was evaluated first: it removes the title
// bar and keeps rounded corners and resize edges, but it gives no caption semantics (no
// HTCAPTION: no Snap-by-drag, double-click or Aero Shake), no minimize/maximize animation
// (WS_CAPTION/WS_MINIMIZEBOX are dropped) and its frame bookkeeping breaks once those styles
// are added back. So the window is created frameless (Qt::FramelessWindowHint: Qt treats the
// whole window rect as client area, frame margins 0) and this filter turns it back into a real
// Windows 11 window:
//   - WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME on the HWND
//     (re-applied whenever Qt rewrites the style): DWM shadow and rounded corners,
//     minimize/maximize animations, Win+arrows, Alt+Space system menu, taskbar minimize.
//   - WM_NCCALCSIZE: no non-client area at all, so client rect == window rect (what Qt expects
//     for a frameless window). Maximized windows are clamped to the monitor work area.
//   - WM_NCHITTEST: 6 px (DPI scaled) resize edges, and HTCAPTION over the QML title bar strip
//     (property `titleBarHeight` of the root Window) unless the point is over an item that takes
//     clicks. HTCAPTION gives native drag, Snap (edges, Snap Layouts on drag), double-click to
//     maximize, Aero Shake and the right-click system menu. The decision itself is Rust code
//     (velo_hit_test in src/window.rs) so it can be unit tested.
//   - DWM attributes: rounded corners, dark/light frame following the theme, optional Mica.
//
// Non-Windows builds keep only the portable parts (graphics API, screenshot).

#include <QtCore/QAbstractNativeEventFilter>
#include <QtCore/QByteArray>
#include <QtCore/QMetaProperty>
#include <QtCore/QPointer>
#include <QtCore/QRectF>
#include <QtCore/QTimer>
#include <QtCore/QVariant>
#include <QtGui/QGuiApplication>
#include <QtGui/QImage>
#include <QtQml/QQmlApplicationEngine>
#include <QtQuick/QQuickItem>
#include <QtQuick/QQuickWindow>
#include <QtQuick/QSGRendererInterface>

#include <algorithm>
#include <cstdint>

extern "C" void velo_media_window_changed(std::uintptr_t hwnd);

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <dwmapi.h>
#endif

namespace {

constexpr int kResizeBorder = 6;     // resize band inside the client area, logical px
constexpr int kDefaultTitleBar = 44; // Theme.size.titleBar, used if the property is missing

QQuickWindow *rootWindow(QQmlApplicationEngine *engine)
{
    if (!engine)
        return nullptr;
    const auto roots = engine->rootObjects();
    for (QObject *root : roots) {
        if (auto *window = qobject_cast<QQuickWindow *>(root))
            return window;
    }
    return nullptr;
}

// Does this item take clicks, so the title bar must not drag the window over it?
//   - `property bool veloNoDrag` on the item decides explicitly (true: clicks, false: drag);
//   - MouseArea that accepts a button; any Qt Quick Controls control except plain containers
//     (Pane, Page, Frame...); Flickable/ListView; text inputs;
//   - an item with an enabled TapHandler or DragHandler.
// HoverHandler and hover-only MouseAreas do not block dragging.
bool takesClicks(const QQuickItem *item)
{
    const QVariant explicitFlag = item->property("veloNoDrag");
    if (explicitFlag.isValid())
        return explicitFlag.toBool();
    if (item->inherits("QQuickMouseArea"))
        return item->acceptedMouseButtons() != Qt::NoButton;
    if (item->inherits("QQuickControl"))
        return !item->inherits("QQuickPane") && !item->inherits("QQuickContainer");
    if (item->inherits("QQuickFlickable") || item->inherits("QQuickTextInput")
        || item->inherits("QQuickTextEdit"))
        return true;
    const auto children = item->children();
    for (const QObject *child : children) {
        if ((child->inherits("QQuickTapHandler") || child->inherits("QQuickDragHandler"))
            && child->property("enabled").toBool())
            return true;
    }
    return false;
}

bool interactiveAt(QQuickItem *item, const QPointF &scenePos)
{
    if (!item->isVisible() || !item->isEnabled() || item->opacity() <= 0.0)
        return false;
    const bool inside = item->contains(item->mapFromScene(scenePos));
    if (item->clip() && !inside)
        return false;
    if (inside && takesClicks(item))
        return true;
    const auto children = item->childItems();
    for (QQuickItem *child : children) {
        if (interactiveAt(child, scenePos))
            return true;
    }
    return false;
}

} // namespace

#ifdef Q_OS_WIN

extern "C" {
// Mirrors window::HitTestInput (repr(C)) in src/window.rs.
struct VeloHitTestInput {
    std::int32_t x, y, width, height;
    std::int32_t clientLeft, clientTop, clientRight, clientBottom;
    std::int32_t border, caption;
    bool maximized;
    bool overInteractive;
};
std::uint32_t velo_hit_test(const VeloHitTestInput *input);
struct VeloPhysicalRect {
    std::int32_t left, top, right, bottom;
};
VeloPhysicalRect velo_client_insets(const VeloPhysicalRect *window,
                                  const VeloPhysicalRect *work, bool maximized);
}

namespace {

constexpr LONG_PTR kChromeStyle = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME;

class WindowChrome final : public QAbstractNativeEventFilter
{
public:
    WindowChrome(QQuickWindow *window, bool mica)
        : m_window(window)
        , m_hwnd(reinterpret_cast<HWND>(window->winId()))
        , m_mica(mica)
        // With the whole client area as glass (Mica), DWM paints its own caption buttons
        // over the window when WS_SYSMENU is set; Mica mode gives up the system menu.
        , m_style(mica ? kChromeStyle & ~static_cast<LONG_PTR>(WS_SYSMENU) : kChromeStyle)
    {
    }

    void apply()
    {
        const LONG_PTR style = GetWindowLongPtrW(m_hwnd, GWL_STYLE);
        const LONG_PTR removed = m_mica ? static_cast<LONG_PTR>(WS_SYSMENU) : 0;
        SetWindowLongPtrW(m_hwnd, GWL_STYLE, (style | m_style) & ~removed);

        // A non-zero glass margin keeps the DWM shadow on a window without non-client area.
        // With Mica the whole client is glass so the backdrop shows wherever QML is transparent.
        syncFrame();

        const DWM_WINDOW_CORNER_PREFERENCE corners = DWMWCP_ROUND;
        DwmSetWindowAttribute(m_hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corners, sizeof corners);
        if (m_mica) {
            const DWM_SYSTEMBACKDROP_TYPE backdrop = DWMSBT_MAINWINDOW;
            DwmSetWindowAttribute(m_hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop, sizeof backdrop);
        }
        syncDarkFrame();

        SetWindowPos(m_hwnd, nullptr, 0, 0, 0, 0,
                     SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
                         | SWP_NOOWNERZORDER);
    }

    // Dark DWM frame (border tint, Mica tone, system menu) when the theme is dark.
    void syncDarkFrame()
    {
        if (!m_window)
            return;
        const BOOL dark = m_window->property("darkChrome").toBool() ? TRUE : FALSE;
        if (dark == m_dark)
            return;
        m_dark = dark;
        DwmSetWindowAttribute(m_hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark, sizeof dark);
        if (m_mica) {
            const DWM_SYSTEMBACKDROP_TYPE backdrop = DWMSBT_MAINWINDOW;
            DwmSetWindowAttribute(m_hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop, sizeof backdrop);
        }
    }

    bool nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result) override
    {
        if (eventType != "windows_generic_MSG")
            return false;
        auto *msg = static_cast<MSG *>(message);
        if (msg->hwnd != m_hwnd || !m_window)
            return false;

        switch (msg->message) {
        case WM_NCCALCSIZE: {
            // RECT en la forma simple; rgrc[0] en la forma completa (mismo primer campo).
            auto *rect = reinterpret_cast<RECT *>(msg->lParam);
            if (IsZoomed(m_hwnd))
                clampToWorkArea(*rect);
            *result = 0;
            return true;
        }
        case WM_SIZE:
            syncFrame();
            return false;
        case WM_NCHITTEST:
            *result = hitTest(msg->lParam);
            return true;
        case WM_GETMINMAXINFO: {
            // El HWND conserva WS_THICKFRAME: maximizar necesita el marco invisible fuera
            // de rcWork. NCCALCSIZE lo quita del cliente, en píxeles físicos del monitor.
            auto *info = reinterpret_cast<MINMAXINFO *>(msg->lParam);
            MONITORINFO monitor{sizeof(MONITORINFO)};
            if (GetMonitorInfoW(MonitorFromWindow(m_hwnd, MONITOR_DEFAULTTONEAREST), &monitor)) {
                const UINT dpi = GetDpiForWindow(m_hwnd);
                const int padded = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                const int frameX = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + padded;
                const int frameY = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + padded;
                info->ptMaxPosition.x = monitor.rcWork.left - monitor.rcMonitor.left - frameX;
                info->ptMaxPosition.y = monitor.rcWork.top - monitor.rcMonitor.top - frameY;
                info->ptMaxSize.x = monitor.rcWork.right - monitor.rcWork.left + 2 * frameX;
                info->ptMaxSize.y = monitor.rcWork.bottom - monitor.rcWork.top + 2 * frameY;
                const qreal dpr = m_window->devicePixelRatio();
                info->ptMinTrackSize.x = std::max(info->ptMinTrackSize.x,
                                                 static_cast<LONG>(qRound(m_window->minimumWidth() * dpr)));
                info->ptMinTrackSize.y = std::max(info->ptMinTrackSize.y,
                                                 static_cast<LONG>(qRound(m_window->minimumHeight() * dpr)));
                info->ptMaxTrackSize.x = std::min(info->ptMaxTrackSize.x,
                                                 static_cast<LONG>(qRound(m_window->maximumWidth() * dpr)));
                info->ptMaxTrackSize.y = std::min(info->ptMaxTrackSize.y,
                                                 static_cast<LONG>(qRound(m_window->maximumHeight() * dpr)));
                *result = 0;
                return true; // Qt no debe volver a sustituirlo por geometría del principal.
            }
            return false;
        }
        case WM_STYLECHANGING:
            // Qt recomputes the style on flag and state changes; keep the frame bits.
            if (msg->wParam == static_cast<WPARAM>(GWL_STYLE)) {
                auto *change = reinterpret_cast<STYLESTRUCT *>(msg->lParam);
                if (!(change->styleNew & WS_CHILD))
                    change->styleNew |= static_cast<DWORD>(m_style);
            }
            return false;
        default:
            return false;
        }
    }

private:
    void syncFrame() const
    {
        const bool maximized = IsZoomed(m_hwnd) != FALSE;
        // El margen de sombra de 1 px y el borde DWM no deben pintar una línea al maximizar.
        const MARGINS glass = m_mica ? MARGINS{-1, -1, -1, -1}
                                    : (maximized ? MARGINS{0, 0, 0, 0} : MARGINS{0, 0, 1, 0});
        DwmExtendFrameIntoClientArea(m_hwnd, &glass);
        const COLORREF border = maximized ? DWMWA_COLOR_NONE : DWMWA_COLOR_DEFAULT;
        DwmSetWindowAttribute(m_hwnd, DWMWA_BORDER_COLOR, &border, sizeof border);
    }

    void clampToWorkArea(RECT &rect) const
    {
        MONITORINFO monitor{sizeof(MONITORINFO)};
        // El rectángulo propuesto ya puede estar en otro monitor durante Win+Shift+flecha.
        if (!GetMonitorInfoW(MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST), &monitor))
            return;
        const VeloPhysicalRect window{rect.left, rect.top, rect.right, rect.bottom};
        const VeloPhysicalRect work{monitor.rcWork.left, monitor.rcWork.top,
                                    monitor.rcWork.right, monitor.rcWork.bottom};
        const auto insets = velo_client_insets(&window, &work, true);
        rect.left += insets.left;
        rect.top += insets.top;
        rect.right -= insets.right;
        rect.bottom -= insets.bottom;
    }

    LRESULT hitTest(LPARAM lParam) const
    {
        const POINT screen{static_cast<short>(LOWORD(lParam)), static_cast<short>(HIWORD(lParam))};
        RECT win{};
        RECT client{};
        GetWindowRect(m_hwnd, &win);
        GetClientRect(m_hwnd, &client);
        POINT origin{0, 0};
        ClientToScreen(m_hwnd, &origin);

        const qreal dpr = m_window->devicePixelRatio();
        const QVariant titleBar = m_window->property("titleBarHeight");
        const int titleBarLogical = titleBar.isValid() ? titleBar.toInt() : kDefaultTitleBar;

        VeloHitTestInput in{};
        in.x = screen.x - win.left;
        in.y = screen.y - win.top;
        in.width = win.right - win.left;
        in.height = win.bottom - win.top;
        in.clientLeft = origin.x - win.left;
        in.clientTop = origin.y - win.top;
        in.clientRight = in.clientLeft + (client.right - client.left);
        in.clientBottom = in.clientTop + (client.bottom - client.top);
        in.border = qRound(kResizeBorder * dpr);
        in.caption = qRound(titleBarLogical * dpr);
        in.maximized = IsZoomed(m_hwnd) != FALSE;

        // Only walk the scene for points inside the title bar strip.
        const int clientY = screen.y - origin.y;
        if (clientY >= 0 && clientY < in.caption) {
            const QPointF scenePos((screen.x - origin.x) / dpr, clientY / dpr);
            // https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-snap-layout-menu
            // QRectF is in QML scene pixels, so zoom/DPI changes need no native constants.
            const QRectF maximize = m_window->property("veloMaximizeButtonRect").toRectF();
            in.overInteractive = interactiveAt(m_window->contentItem(), scenePos);
            if (maximize.contains(scenePos) && (GetWindowLongPtrW(m_hwnd, GWL_STYLE) & WS_MAXIMIZEBOX)) {
                in.overInteractive = true;
                // Keep Rust's existing priority for interactive caption items.
                if (velo_hit_test(&in) == HTCLIENT) return HTMAXBUTTON;
            }
        }
        return static_cast<LRESULT>(velo_hit_test(&in));
    }

    QPointer<QQuickWindow> m_window;
    HWND m_hwnd;
    bool m_mica;
    LONG_PTR m_style;
    BOOL m_dark = -1;
};

} // namespace

#endif // Q_OS_WIN

extern "C" void velo_window_prepare(bool mica)
{
#ifdef Q_OS_WIN
    // Hardware scene graph on Direct3D 11 (Qt's default on Windows, stated explicitly).
    QQuickWindow::setGraphicsApi(QSGRendererInterface::Direct3D11);
#endif
    if (mica)
        QQuickWindow::setDefaultAlphaBuffer(true);
}

extern "C" bool velo_window_chrome_install(QQmlApplicationEngine *engine, bool mica)
{
    QQuickWindow *window = rootWindow(engine);
    if (!window)
        return false;
    window->setProperty("mica", mica);

#ifdef Q_OS_WIN
    // Lives as long as the window; removed and deleted with it.
    auto *chrome = new WindowChrome(window, mica);
    qGuiApp->installNativeEventFilter(chrome);
    chrome->apply();
    velo_media_window_changed(static_cast<std::uintptr_t>(window->winId()));
    QObject::connect(window, &QQuickWindow::colorChanged, window, [chrome] { chrome->syncDarkFrame(); });
    QObject::connect(window, &QWindow::activeChanged, window, [chrome] { chrome->syncDarkFrame(); });
    // QML declares darkChrome dynamically: obtain its actual notification signal rather
    // than relying on color/focus changes (Mica keeps the window color transparent).
    const int darkChromeIndex = window->metaObject()->indexOfProperty("darkChrome");
    if (darkChromeIndex >= 0) {
        const QMetaProperty darkChrome = window->metaObject()->property(darkChromeIndex);
        if (darkChrome.hasNotifySignal()) {
            QMetaObject::connect(window, darkChrome.notifySignal(), window,
                                 [chrome] { chrome->syncDarkFrame(); });
        }
    }
    QObject::connect(window, &QObject::destroyed, qGuiApp, [chrome] {
        velo_media_window_changed(0);
        qGuiApp->removeNativeEventFilter(chrome);
        delete chrome;
    });
#endif

    const QByteArray shotPath = qgetenv("VELO_SCREENSHOT");
    if (!shotPath.isEmpty()) {
        QPointer<QQuickWindow> target(window);
        QTimer::singleShot(1500, window, [target, shotPath] {
            if (target)
                target->grabWindow().save(QString::fromLocal8Bit(shotPath));
        });
    }
    return true;
}
