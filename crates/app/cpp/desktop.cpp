// Bandeja nativa Windows con QGuiApplication: sin QApplication ni QtWidgets.
#include <QtCore/QCoreApplication>
#include <QtCore/QEvent>
#include <QtCore/QPointer>
#include <QtCore/QVariant>
#include <QtCore/QRectF>
#include <cmath>
#include <QtGui/QCloseEvent>
#include <QtGui/QGuiApplication>
#include <QtGui/QSessionManager>
#include <QtQml/QQmlApplicationEngine>
#include <QtQuick/QQuickWindow>
#include <cstdint>
#include <cstring>

extern "C" void velo_desktop_action(std::uint32_t action);
extern "C" bool velo_should_hide_on_close(bool preference, bool trayAvailable, bool quitting);

#ifdef Q_OS_WIN
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <shellapi.h>
#include <shobjidl.h>
#include <array>
#include <QtCore/QAbstractNativeEventFilter>
#include <QtCore/QSettings>
#include <QtGui/QImage>
#include <QtGui/QImageReader>
#include <QtGui/QPainter>
#include <QtGui/QIcon>
#include <vector>

namespace {
constexpr UINT kTrayMessage = WM_APP + 103;

// Rasteriza una vez el símbolo (logo o botón) a tamaño de icono pequeño.
HICON makeIcon(const QString &resource = QStringLiteral("velo_mono.svg"), bool white = false)
{
    const int side = GetSystemMetrics(SM_CXSMICON);
    // La bandeja usa el logo monocromo de Velo; el SVG se rasteriza ya a su tamaño.
    QImageReader reader(QStringLiteral(":/qt/qml/Velo/icons/") + resource);
    reader.setScaledSize(QSize(side, side));
    const QImage source = reader.read();
    if (source.isNull()) return CopyIcon(LoadIconW(nullptr, MAKEINTRESOURCEW(32512)));
    QImage image(side, side, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    painter.drawImage(image.rect(), source);
    // La bandeja sigue el tema del sistema (puede diferir del tema elegido en Velo).
    const QSettings personalization(QStringLiteral("HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), QSettings::NativeFormat);
    const bool light = personalization.value(QStringLiteral("SystemUsesLightTheme"), 0).toBool();
    painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
    painter.fillRect(image.rect(), (!white && light) ? Qt::black : Qt::white);
    painter.end();
    BITMAPV5HEADER header{};
    header.bV5Size = sizeof(header);
    header.bV5Width = side;
    header.bV5Height = -side;
    header.bV5Planes = 1;
    header.bV5BitCount = 32;
    header.bV5Compression = BI_BITFIELDS;
    header.bV5RedMask = 0x00ff0000;
    header.bV5GreenMask = 0x0000ff00;
    header.bV5BlueMask = 0x000000ff;
    header.bV5AlphaMask = 0xff000000;
    void *bits = nullptr;
    HDC dc = GetDC(nullptr);
    HBITMAP color = CreateDIBSection(dc, reinterpret_cast<BITMAPINFO *>(&header), DIB_RGB_COLORS, &bits, nullptr, 0);
    ReleaseDC(nullptr, dc);
    if (!color || !bits) { if (color) DeleteObject(color); return nullptr; }
    memcpy(bits, image.constBits(), side * side * 4);
    // Máscara a cero (opaca): con color de 32 bits Windows usa el alfa del color.
    const std::vector<BYTE> zeros(static_cast<size_t>((side + 15) / 16 * 2 * side), 0);
    HBITMAP mask = CreateBitmap(side, side, 1, 1, zeros.data());
    ICONINFO info{};
    info.fIcon = TRUE;
    info.hbmColor = color;
    info.hbmMask = mask;
    HICON icon = mask ? CreateIconIndirect(&info) : nullptr;
    DeleteObject(color);
    if (mask) DeleteObject(mask);
    return icon;
}

class Tray final : public QObject, public QAbstractNativeEventFilter {
public:
    explicit Tray(QQuickWindow *window) : QObject(window), m_window(window), m_restart(RegisterWindowMessageW(L"TaskbarCreated")), m_buttonCreated(RegisterWindowMessageW(L"TaskbarButtonCreated"))
    {
        m_comInitialized = SUCCEEDED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED));
        const std::array<QString, 6> symbols = {QStringLiteral("favorite.svg"), QStringLiteral("favorite_fill.svg"),
            QStringLiteral("skip_previous.svg"), QStringLiteral("play_arrow.svg"), QStringLiteral("pause.svg"), QStringLiteral("skip_next.svg")};
        for (size_t i = 0; i < symbols.size(); ++i) m_buttonsIcons[i] = makeIcon(symbols[i], true);
        m_data.cbSize = sizeof(m_data);
        m_data.hWnd = reinterpret_cast<HWND>(window->winId());
        m_data.uID = 1;
        m_data.uCallbackMessage = kTrayMessage;
        m_data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        m_data.hIcon = makeIcon();
        update(QCoreApplication::applicationName(), false, false);
        add();
        window->installEventFilter(this);
        qGuiApp->installNativeEventFilter(this);
        QObject::connect(window, &QWindow::visibilityChanged, this, [this](QWindow::Visibility visibility) {
            if (visibility == QWindow::Maximized || visibility == QWindow::Windowed)
                m_restoreMaximized = visibility == QWindow::Maximized;
        });
        QObject::connect(qGuiApp, &QGuiApplication::commitDataRequest, this, [this](QSessionManager &) { m_quitting = true; });
        QObject::connect(qGuiApp, &QCoreApplication::aboutToQuit, this, [this] { m_quitting = true; remove(); });
    }
    ~Tray() override {
        remove(); qGuiApp->removeNativeEventFilter(this);
        if (m_taskbar) m_taskbar->Release();
        for (HICON icon : m_buttonsIcons) if (icon) DestroyIcon(icon);
        if (m_data.hIcon) DestroyIcon(m_data.hIcon);
        if (m_comInitialized) CoUninitialize();
    }

    void show()
    {
        if (!m_window) return;
        if (m_window->isVisible() && m_window->visibility() == QWindow::Minimized) {
            // SW_RESTORE devuelve la ventana a su estado previo (normal o maximizada) con la
            // animación nativa de Windows.
            ShowWindow(reinterpret_cast<HWND>(m_window->winId()), SW_RESTORE);
        } else if (!m_window->isVisible()) {
            if (m_restoreMaximized) m_window->showMaximized(); else m_window->showNormal();
        }
        m_window->raise();
        m_window->requestActivate();
    }

    // 1 minimizar, 2 maximizar, 3 restaurar, 4 alternar maximizado. Va por WM_SYSCOMMAND (lo mismo
    // que el menú del sistema y el botón nativo) para que DWM anime la transición; la ventana
    // conserva WS_CAPTION|WS_THICKFRAME|WS_MINIMIZEBOX|WS_MAXIMIZEBOX (ver window_chrome.cpp).
    void command(std::uint32_t cmd)
    {
        if (!m_window) return;
        const HWND hwnd = reinterpret_cast<HWND>(m_window->winId());
        WPARAM sc = 0;
        switch (cmd) {
        case 1: sc = SC_MINIMIZE; break;
        case 2: sc = SC_MAXIMIZE; break;
        case 3: sc = SC_RESTORE; break;
        case 4: sc = IsZoomed(hwnd) ? SC_RESTORE : SC_MAXIMIZE; break;
        default: return;
        }
        // Post: la animación no debe correr dentro del manejador del clic de QML.
        PostMessageW(hwnd, WM_SYSCOMMAND, sc, 0);
    }

    void update(const QString &tooltip, bool playing, bool controllable, bool liked = false, bool likeAvailable = false)
    {
        const bool buttonsChanged = m_playing != playing || m_controllable != controllable
            || m_liked != liked || m_likeAvailable != likeAvailable;
        m_liked = liked; m_likeAvailable = likeAvailable;
        m_playing = playing;
        m_controllable = controllable;
        if (buttonsChanged && m_buttonsAdded) updateButtons(false);
        QString text = tooltip.left(127);
        if (!text.isEmpty() && text.back().isHighSurrogate()) text.chop(1);
        if (m_tooltip == text) return;
        m_tooltip = text;
        text.toWCharArray(m_data.szTip);
        m_data.szTip[text.size()] = L'\0';
        if (m_available) Shell_NotifyIconW(NIM_MODIFY, &m_data);
    }

    bool eventFilter(QObject *object, QEvent *event) override
    {
        if (object == m_window && event->type() == QEvent::Close
            && velo_should_hide_on_close(m_window->property("closeToTray").toBool(), m_available, m_quitting)) {
            static_cast<QCloseEvent *>(event)->ignore();
            m_window->hide();
            return true;
        }
        return false;
    }

    bool nativeEventFilter(const QByteArray &type, void *message, qintptr *result) override
    {
        if (type != "windows_generic_MSG") return false;
        auto *msg = static_cast<MSG *>(message);
        if (msg->hwnd != m_data.hWnd) return false;
        if (msg->message == WM_SETTINGCHANGE) {
            if (HICON icon = makeIcon()) {
                HICON old = m_data.hIcon;
                m_data.hIcon = icon;
                if (m_available) Shell_NotifyIconW(NIM_MODIFY, &m_data);
                if (old) DestroyIcon(old);
            }
            return false;
        }
        if (msg->message == m_buttonCreated && m_buttonCreated != 0) {
            // Explorer sends this again after restart. Discard its old COM proxy and rebuild once.
            m_buttonsAdded = false;
            if (m_taskbar) { m_taskbar->Release(); m_taskbar = nullptr; }
            if (SUCCEEDED(CoCreateInstance(CLSID_TaskbarList, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&m_taskbar)))) {
                if (SUCCEEDED(m_taskbar->HrInit())) updateButtons(true);
                else { m_taskbar->Release(); m_taskbar = nullptr; }
            }
            return false;
        }
        if (msg->message == WM_COMMAND && HIWORD(msg->wParam) == THBN_CLICKED) {
            const UINT id = LOWORD(msg->wParam);
            if (id >= 101 && id <= 104) {
                if (id == 101) { if (m_likeAvailable) velo_desktop_action(6); }
                else if (m_controllable) velo_desktop_action(id == 102 ? 3 : (id == 103 ? 1 : 2));
                *result = 0; return true;
            }
        }
        if (msg->message == m_restart && m_restart != 0) { m_available = false; add(); return false; }
        if (msg->message != kTrayMessage) return false;
        switch (LOWORD(msg->lParam)) {
        case NIN_SELECT:
        case NIN_KEYSELECT: show(); break;
        case WM_CONTEXTMENU: menu(); break;
        default: break;
        }
        *result = 0;
        return true;
    }
private:
    void updateButtons(bool addButtons)
    {
        if (!m_taskbar) return;
        THUMBBUTTON buttons[4]{};
        const HICON icons[4] = {m_buttonsIcons[m_liked ? 1 : 0], m_buttonsIcons[2], m_buttonsIcons[m_playing ? 4 : 3], m_buttonsIcons[5]};
        const QString tips[4] = {QCoreApplication::translate("Desktop", m_liked ? "Quitar de Me gusta" : "Me gusta"),
            QCoreApplication::translate("Desktop", "Anterior"), QCoreApplication::translate("Desktop", m_playing ? "Pausa" : "Reproducir"),
            QCoreApplication::translate("Desktop", "Siguiente")};
        for (UINT i = 0; i < 4; ++i) {
            auto &b = buttons[i]; b.dwMask = THB_ICON | THB_TOOLTIP | THB_FLAGS; b.iId = 101 + i; b.hIcon = icons[i];
            b.dwFlags = (i == 0 ? m_likeAvailable : m_controllable) ? THBF_ENABLED : THBF_DISABLED;
            const QString tip = tips[i].left(259); tip.toWCharArray(b.szTip); b.szTip[tip.size()] = L'\0';
        }
        if (addButtons) m_buttonsAdded = SUCCEEDED(m_taskbar->ThumbBarAddButtons(m_data.hWnd, 4, buttons));
        else m_taskbar->ThumbBarUpdateButtons(m_data.hWnd, 4, buttons);
    }
    void add()
    {
        m_available = m_data.hIcon && Shell_NotifyIconW(NIM_ADD, &m_data);
        if (m_available) { m_data.uVersion = NOTIFYICON_VERSION_4; Shell_NotifyIconW(NIM_SETVERSION, &m_data); }
        m_window->setProperty("trayAvailable", m_available);
    }
    void remove() { if (m_available) Shell_NotifyIconW(NIM_DELETE, &m_data); m_available = false; }
    void menu()
    {
        HMENU menu = CreatePopupMenu();
        if (!menu) return;
        const UINT playback = MF_STRING | (m_controllable ? MF_ENABLED : MF_GRAYED);
        const auto label = [](const QString &text) { return text.toStdWString(); };
        AppendMenuW(menu, playback, 1, label(m_playing ? QCoreApplication::translate("Desktop", "Pausa") : QCoreApplication::translate("Desktop", "Reproducir")).c_str());
        AppendMenuW(menu, playback, 2, label(QCoreApplication::translate("Desktop", "Siguiente")).c_str());
        AppendMenuW(menu, playback, 3, label(QCoreApplication::translate("Desktop", "Anterior")).c_str());
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, 4, label(QCoreApplication::translate("Desktop", "Mostrar Velo")).c_str());
        AppendMenuW(menu, MF_STRING, 5, label(QCoreApplication::translate("Desktop", "Salir")).c_str());
        POINT cursor{};
        GetCursorPos(&cursor);
        SetForegroundWindow(m_data.hWnd);
        const UINT action = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, cursor.x, cursor.y, 0, m_data.hWnd, nullptr);
        PostMessageW(m_data.hWnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if (action == 4) show();
        else if (action == 5) { m_quitting = true; qGuiApp->quit(); }
        else if (action != 0) velo_desktop_action(action);
    }
    QPointer<QQuickWindow> m_window;
    NOTIFYICONDATAW m_data{};
    UINT m_restart, m_buttonCreated;
    ITaskbarList3 *m_taskbar = nullptr;
    std::array<HICON, 6> m_buttonsIcons{};
    bool m_buttonsAdded = false, m_comInitialized = false, m_liked = false, m_likeAvailable = false;
    QString m_tooltip;
    bool m_available = false, m_playing = false, m_controllable = false;
    bool m_quitting = false, m_restoreMaximized = false;
};
QPointer<Tray> tray;
}
#endif

extern "C" void velo_desktop_install(QQmlApplicationEngine *engine)
{
#ifdef Q_OS_WIN
    for (QObject *root : engine->rootObjects()) {
        if (auto *window = qobject_cast<QQuickWindow *>(root)) {
            window->setIcon(QIcon(QStringLiteral(":/qt/qml/Velo/icons/velo.svg")));
            tray = new Tray(window);
            break;
        }
    }
#else
    Q_UNUSED(engine);
#endif
}

extern "C" void velo_desktop_show()
{
#ifdef Q_OS_WIN
    if (tray) tray->show();
#else
    for (QWindow *window : qGuiApp->topLevelWindows()) { window->show(); window->raise(); window->requestActivate(); break; }
#endif
}

extern "C" void velo_window_command(std::uint32_t cmd)
{
#ifdef Q_OS_WIN
    if (tray) { tray->command(cmd); return; }
#endif
    for (QWindow *window : qGuiApp->topLevelWindows()) {
        switch (cmd) {
        case 1: window->showMinimized(); break;
        case 2: window->showMaximized(); break;
        case 3: window->showNormal(); break;
        case 4: if (window->visibility() == QWindow::Maximized) window->showNormal(); else window->showMaximized(); break;
        default: break;
        }
        break;
    }
}

// QML reports scene coordinates (logical pixels), on geometry/style changes only.
extern "C" void velo_set_maximize_button_rect(double x, double y, double width, double height)
{
    const bool valid = std::isfinite(x) && std::isfinite(y) && std::isfinite(width)
        && std::isfinite(height) && width > 0 && height > 0;
    for (QWindow *window : qGuiApp->topLevelWindows()) {
        if (auto *quick = qobject_cast<QQuickWindow *>(window)) {
            quick->setProperty("veloMaximizeButtonRect", valid ? QRectF(x, y, width, height) : QRectF());
            break;
        }
    }
}

extern "C" void velo_desktop_update(const char *tooltip, bool playing, bool controllable, bool liked, bool likeAvailable)
{
#ifdef Q_OS_WIN
    if (tray) tray->update(QString::fromUtf8(tooltip), playing, controllable, liked, likeAvailable);
#else
    Q_UNUSED(tooltip); Q_UNUSED(playing); Q_UNUSED(controllable); Q_UNUSED(liked); Q_UNUSED(likeAvailable);
#endif
}
