import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Controls.impl
import QtQuick.Effects
import QtQuick.Templates as T
import Velo.Backend as Backend
import "../../theme"

Item {
    id: page
    signal navigate(string kind, string itemId, string owner)
    signal queueRequested()
    Keys.onShortcutOverride: event => {
        const w = page.Window.window
        if (event.key === Qt.Key_Space && w && w.activeFocusItem && w.activeFocusItem.activeFocusOnTab)
            event.accepted = true
    }
    readonly property var window: page.Window.window
    readonly property bool effectsAllowed: visible && Theme.motion.scale > 0   // regla 1: interacción, no depende de que suene música
        && (!window || (window.visible && window.visibility !== Window.Minimized
            && (window.active || !Theme.pauseEffectsInBackground)))
    // H1: "Buenos días / Buenas tardes / Buenas noches", recomputed whenever Inicio is shown.
    property string greeting: ""
    function updateGreeting() {
        const h = new Date().getHours()
        greeting = h >= 5 && h < 12 ? qsTr("Buenos días") : h >= 12 && h < 20 ? qsTr("Buenas tardes") : qsTr("Buenas noches")
    }
    Component.onCompleted: updateGreeting()
    onVisibleChanged: { if (visible) updateGreeting() }

    // R3 (P2-3): while the first data of Inicio is on its way nothing says «you have nothing»:
    // a static skeleton stands in (no shimmer, nothing runs while it waits) and every section
    // stays hidden until it has rows; a section that ends up empty never shows.
    readonly property bool hasContent: quickRepeater.count > 0 || madeList.count > 0 || mixList.count > 0
                                       || recentList.count > 0 || playlistList.count > 0 || artistList.count > 0
                                       || topList.count > 0 || recList.count > 0 || Backend.Player.djAvailable
    readonly property bool firstLoad: !hasContent && (Backend.Library.madeForYouStatus === "loading"
                                                      || Backend.Library.sidebarStatus === "loading")
    property string loadError: ""
    onFirstLoadChanged: if (firstLoad) loadError = ""
    onHasContentChanged: if (hasContent) loadError = ""
    Connections {
        target: Backend.Player
        function onError(message) {
            if (page.visible && !page.hasContent) page.loadError = message
        }
    }

    // R3 (P2-6): section titles are headings for Narrador (jump from section to section).
    component SectionHeading: LibraryLabel {
        font.pixelSize: Theme.type.heading
        font.weight: Theme.type.semibold
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }

    // Shelves advance immediately; scrolling is layout, never an animated property.
    // The vertical wheel scrolls the page; a horizontal gesture belongs to the shelf.
    component ShelfArrow: T.AbstractButton {
        id: arrow
        property bool forward: true
        property bool shown: false
        implicitWidth: Theme.library.shelfArrow
        implicitHeight: Theme.library.shelfArrow
        hoverEnabled: true
        focusPolicy: Qt.NoFocus
        enabled: shown
        opacity: shown ? 1 : 0
        Behavior on opacity {
            enabled: page.effectsAllowed
            OpacityAnimator { id: arrowFade; duration: Math.max(1, arrow.shown ? Theme.motion.fast : Theme.motion.instant); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        scale: pressed ? Theme.feel.pressScale : 1
        Behavior on scale {
            enabled: page.effectsAllowed
            ScaleAnimator { id: arrowScale; duration: arrow.pressed ? Theme.motion.instant : Theme.motion.fast; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
        }
        Connections {
            target: page
            function onEffectsAllowedChanged() {
                if (!page.effectsAllowed) { arrowFade.complete(); arrowScale.complete() }
            }
        }
        Accessible.role: Accessible.Button
        Accessible.name: text
        text: forward ? qsTr("Ver más a la derecha") : qsTr("Ver más a la izquierda")
        background: Rectangle {
            radius: height / 2
            color: arrow.hovered ? Theme.surface3 : Theme.surface1
            border.width: Theme.stroke.hairline
            border.color: Theme.hairline
            RectangularShadow {
                z: -1
                anchors.fill: parent
                radius: parent.radius
                offset.y: Theme.elevation.raised.offsetY
                blur: Theme.elevation.raised.blur
                spread: Theme.elevation.raised.spread
                color: Theme.library.popoverShadow
            }
        }
        contentItem: Item {
            IconImage {
                anchors.centerIn: parent
                source: Qt.resolvedUrl("../../icons/chevron_right.svg")
                sourceSize: Qt.size(Theme.icon.md, Theme.icon.md)
                rotation: arrow.forward ? 0 : 180
                color: Theme.text
            }
        }
    }
    component ShelfArrows: Item {
        id: arrows
        property ListView list: null
        property Flickable pageViewport: null
        property bool hovered: false
        // Vertical centre of the arrows inside the list (the art of a card, by default).
        property real centerY: Theme.library.cardWidth / 2
        property real step: Theme.library.cardWidth
        // Lives on the list itself (not its scrolling content), so it stays put while the cards
        // move, and the section's Column does not lay it out.
        parent: list
        width: list ? list.width : 0
        height: list ? list.height : 0
        function advance(direction) {
            const l = arrows.list
            const unit = step + l.spacing
            const by = Math.max(unit, Math.floor(l.width * Theme.library.shelfPage / unit) * unit)
            const min = l.originX, max = l.originX + Math.max(0, l.contentWidth - l.width)
            const to = Math.max(min, Math.min(max, l.contentX + direction * by))
            l.contentX = to
        }
        WheelHandler {
            target: null
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: event => {
                if (!arrows.pageViewport || (event.angleDelta.y === 0 && event.pixelDelta.y === 0)) {
                    event.accepted = false
                    return
                }
                const v = arrows.pageViewport
                const delta = event.pixelDelta.y !== 0 ? event.pixelDelta.y
                    : event.angleDelta.y / 120 * Theme.library.wheelScroll
                v.contentY = Math.max(v.originY, Math.min(v.originY + Math.max(0, v.contentHeight - v.height), v.contentY - delta))
                event.accepted = true
            }
        }
        ShelfArrow {
            objectName: "shelfBack"
            forward: false
            x: Theme.space.sm
            y: arrows.centerY - height / 2
            shown: arrows.hovered && arrows.list !== null && !arrows.list.atXBeginning
            onClicked: arrows.advance(-1)
        }
        ShelfArrow {
            objectName: "shelfForward"
            forward: true
            x: arrows.width - width - Theme.space.sm
            y: arrows.centerY - height / 2
            shown: arrows.hovered && arrows.list !== null && !arrows.list.atXEnd
            onClicked: arrows.advance(1)
        }
    }

    // Motion (S1-2, C4): each section's cards enter when its data is there: opacity 0 -> 1 and a
    // 12 px rise, `base`, ease-out, 40 ms apart (the first 8; the rest without delay). Once per
    // opening of Inicio (the page is rebuilt each time): cards made later by scrolling or a
    // refresh simply appear. Clicks are never blocked (an entering card is already live).
    //
    // Gate of one section: arms the moment its first rows exist; only delegates created within
    // `motion.slow` of that moment animate.
    component RevealGate: QtObject {
        property int count: 0
        property real armedAt: 0
        function arm() { if (armedAt === 0) armedAt = Date.now() }
        onCountChanged: if (count > 0) arm()
        Component.onCompleted: if (count > 0) arm()
        // Stagger delay for the item at `index`, or -1 when it should just be there.
        function delayFor(index) {
            arm()
            if (Theme.motion.base <= 0 || Date.now() - armedAt > Theme.motion.slow)
                return -1
            return index < Theme.entrance.staggerMax ? index * Theme.entrance.stagger : 0
        }
    }
    // One card's entrance. The first Animator holds the card at opacity 0 through its stagger
    // delay (render thread, no Timer); then it fades to its resting opacity (kept bound: the
    // Animators write back from C++, which leaves the card's own `enabled` binding in place)
    // while its Translate rises. One shot; a reused delegate jumps to the end.
    component Reveal: SequentialAnimation {
        id: reveal
        property Item item: null
        property Translate shift: null
        property RevealGate gate: null
        property int index: 0
        property real rest: 1
        property bool allowed: page.effectsAllowed
        onAllowedChanged: if (!allowed) settle()
        function begin() {
            if (!allowed) return
            const delay = gate && item && shift ? gate.delayFor(index) : -1
            if (delay < 0)
                return
            rest = item.opacity
            hold.duration = delay
            shift.y = Theme.entrance.cardShift
            restart()
        }
        function settle() { if (running) complete() }
        OpacityAnimator { id: hold; target: reveal.item; from: 0; to: 0 }
        ParallelAnimation {
            OpacityAnimator {
                target: reveal.item; from: 0; to: reveal.rest
                duration: Math.max(1, Theme.motion.base); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut
            }
            NumberAnimation {
                target: reveal.shift; property: "y"; from: Theme.entrance.cardShift; to: 0
                duration: Theme.motion.base; easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut
            }
        }
    }
    // Keep vertical clipping while giving edge cards their full hover surface.
    // Titles and artwork keep their existing page coordinates.
    Flickable {
        id: viewport
        objectName: "homeCardViewport"
        x: -Theme.library.cardBleed
        width: page.width + 2 * Theme.library.cardBleed
        height: page.height
        contentWidth: width
        flickableDirection: Flickable.VerticalFlick
        function revealCard(list, index) {
            list.positionViewAtIndex(index, ListView.Contain)
            const top = list.mapToItem(viewport.contentItem, 0, 0).y
            const bottom = top + list.height
            const target = top < contentY ? top : bottom > contentY + height ? bottom - height : contentY
            // S1: keyboard focus reveals cards immediately, without scroll animation.
            contentY = Math.max(originY, Math.min(target, originY + Math.max(0, contentHeight - height)))
        }
        clip: true
        contentHeight: content.height + Theme.space.xl
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: LibraryScrollBar {}
        Column {
            id: content
            x: Theme.library.cardBleed
            width: page.width - Theme.space.md
            spacing: Theme.space.xl
            // H1: greeting by local time, computed when Inicio opens (no Timer).
            LibraryLabel {
                objectName: "homeGreeting"
                width: parent.width
                text: page.greeting
                font.pixelSize: Theme.type.display
                font.weight: Theme.type.bold
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            // R3 (P2-3): first load: static plates in the shape of the page (quick access tiles,
            // then a shelf of cards), at the disabled opacity, like the sidebar's skeleton.
            Column {
                objectName: "homeSkeleton"
                width: parent.width
                visible: page.firstLoad && page.loadError === ""
                spacing: Theme.space.xl
                opacity: Theme.library.disabledOpacity
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Cargando Inicio")
                Grid {
                    width: parent.width
                    columns: quickGrid.columns
                    columnSpacing: Theme.space.md
                    rowSpacing: Theme.space.md
                    Repeater {
                        model: page.firstLoad ? Theme.library.skeletonTiles * 2 : 0
                        delegate: Rectangle {
                            width: quickGrid.tileWidth
                            height: Theme.home.quickTile
                            radius: Theme.radius.md
                            color: Theme.surface2
                        }
                    }
                }
                Column {
                    width: parent.width
                    spacing: Theme.space.md
                    Rectangle { width: Theme.size.coverMin; height: Theme.space.lg; radius: height / 2; color: Theme.surface2 }
                    Row {
                        width: parent.width
                        height: childrenRect.height
                        clip: true
                        spacing: Theme.space.md
                        Repeater {
                            model: page.firstLoad ? Theme.library.skeletonCards : 0
                            delegate: Column {
                                spacing: Theme.space.sm
                                Rectangle { width: Theme.library.cardWidth - 2 * Theme.space.sm; height: width; radius: Theme.radius.md; color: Theme.surface2 }
                                Rectangle { width: Theme.library.cardWidth / 2; height: Theme.space.md; radius: height / 2; color: Theme.surface2 }
                            }
                        }
                    }
                }
            }
            Column {
                objectName: "homeLoadError"
                width: parent.width
                visible: page.loadError !== ""
                spacing: Theme.space.md
                LibraryLabel {
                    width: parent.width
                    text: page.loadError
                    color: Theme.danger
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.AlertMessage
                }
                LibraryButton {
                    text: qsTr("Volver a intentar")
                    onClicked: { page.loadError = ""; Backend.Library.loadHome() }
                }
            }
            // H1 / S9-N5: "Acceso rápido": the first 8 of Library.quickAccess (Spotify's own
            // shortcuts), 4 columns x 2 rows (2 columns when narrow). Horizontal card, square art
            // at the left (artists too, as in the official app), bold name.
            Grid {
                id: quickGrid
                width: parent.width
                visible: quickRepeater.count > 0
                columns: width >= Theme.home.quickWideWidth ? 4 : 2
                columnSpacing: Theme.space.md
                rowSpacing: Theme.space.md
                readonly property real tileWidth: (width - (columns - 1) * columnSpacing) / columns
                RevealGate { id: quickGate; count: quickRepeater.count }
                Repeater {
                    id: quickRepeater
                    model: Backend.Library.quickAccess
                    delegate: LibraryButton {
                        id: tile
                        required property int index
                        visible: index < Theme.home.quickCount
                        transform: Translate { id: tileRise }
                        Reveal { id: tileReveal; item: tile; shift: tileRise; gate: quickGate; index: tile.index }
                        Component.onCompleted: tileReveal.begin()
                        required property string kind
                        required property string objectId
                        required property string name
                        required property string imageUrl
                        readonly property string label: kind === "liked" ? qsTr("Tus me gusta") : name
                        width: quickGrid.tileWidth
                        height: Theme.home.quickTile
                        leftPadding: 0
                        text: label
                        // S1: the hover fill crossfades over the resting one (opacity: instant in,
                        // fast out), like the other library buttons.
                        background: Rectangle {
                            radius: Theme.radius.md
                            color: Theme.surface2
                            border.width: tile.visualFocus ? Theme.stroke.focus : 0
                            border.color: Theme.focusRing
                            Rectangle {
                                anchors.fill: parent
                                radius: parent.radius
                                color: Theme.surface3
                                opacity: tile.live ? 1 : 0
                                Behavior on opacity {
                                    OpacityAnimator { duration: Math.max(1, tile.live ? Theme.motion.instant : Theme.motion.fast); easing.type: Easing.Bezier; easing.bezierCurve: Theme.motion.easeOut }
                                }
                            }
                        }
                        contentItem: Row {
                            spacing: Theme.space.md
                            LibraryArtwork {
                                visible: tile.kind !== "liked"
                                width: Theme.home.quickTile
                                height: width
                                source: visible ? tile.imageUrl : ""
                                fallback: tile.kind === "artist" ? Qt.resolvedUrl("../../icons/person.svg")
                                          : Qt.resolvedUrl("../../icons/library_music.svg")
                            }
                            // «Tus me gusta»: the gradient tile with the heart, as in Spotify.
                            Rectangle {
                                visible: tile.kind === "liked"
                                width: Theme.home.quickTile
                                height: width
                                radius: Theme.radius.md
                                gradient: Gradient {
                                    orientation: Gradient.Vertical
                                    GradientStop { position: 0; color: Theme.home.likedFrom }
                                    GradientStop { position: 1; color: Theme.home.likedTo }
                                }
                                IconImage {
                                    anchors.centerIn: parent
                                    source: Qt.resolvedUrl("../../icons/favorite_fill.svg")
                                    sourceSize: Qt.size(Theme.icon.lg, Theme.icon.lg)
                                    color: Theme.home.likedInk
                                }
                            }
                            LibraryLabel {
                                anchors.verticalCenter: parent.verticalCenter
                                width: tile.width - Theme.home.quickTile - Theme.space.md - tile.rightPadding
                                text: tile.label
                                font.weight: Theme.type.bold
                                wrapMode: Text.Wrap
                                maximumLineCount: 2
                            }
                        }
                        onClicked: {
                            if (kind === "liked") page.navigate("liked", "", "")
                            else page.navigate(kind, objectId, "")
                        }
                    }
                }
            }
            // H1: "Hecho para ti": your Discover Weekly, Release Radar, Daily Mix and daylist.
            // E5b: led by the DJ Livi card when the account has DJ (absent otherwise).
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: madeList.count > 0 || Backend.Player.djAvailable
                HoverHandler { id: madeHover }
                SectionHeading { text: qsTr("Hecho para ti") }
                RevealGate { id: madeGate; count: madeList.count + (Backend.Player.djAvailable ? 1 : 0) }
                ListView {
                    id: madeList
                    objectName: "home_madeList"
                    x: -Theme.space.sm
                    width: parent.width + 2 * Theme.library.cardBleed
                    height: Theme.library.cardWidth + Theme.size.trackRow
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    model: Backend.Library.madeForYou
                    header: Item {
                        width: Backend.Player.djAvailable ? Theme.library.cardWidth + madeList.spacing : 0
                        height: madeList.height
                        DjCard {
                            id: djCard
                            visible: Backend.Player.djAvailable
                            width: Theme.library.cardWidth
                            transform: Translate { id: djCardRise }
                            Reveal { id: djCardReveal; item: djCard; shift: djCardRise; gate: madeGate; index: 0 }
                            Component.onCompleted: djCardReveal.begin()
                            onActiveFocusChanged: { if (activeFocus) madeList.positionViewAtBeginning() }
                        }
                    }
                    delegate: MediaCard {
                        id: madeCard
                        required property int index
                        transform: Translate { id: madeCardRise }
                        Reveal { id: madeCardReveal; item: madeCard; shift: madeCardRise; gate: madeGate; index: madeCard.index + (Backend.Player.djAvailable ? 1 : 0) }
                        Component.onCompleted: madeCardReveal.begin()
                        ListView.onReused: madeCardReveal.settle()
                        required property string playlistId
                        required property string name
                        required property string description
                        required property string imageUrl
                        width: Theme.library.cardWidth
                        artSource: imageUrl
                        title: name
                        // Spotify descriptions may carry markup: shown as plain text.
                        subtitle: description.replace(/<[^>]*>/g, "")
                        uri: "spotify:playlist:" + playlistId
                        onActiveFocusChanged: { if (activeFocus) viewport.revealCard(madeList, index) }
                        onClicked: page.navigate("playlist", playlistId, "Spotify")
                    }
                }
                ShelfArrows { pageViewport: viewport; list: madeList; hovered: madeHover.hovered }
            }
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: Backend.Library.forYouAvailable && mixList.count > 0
                HoverHandler { id: mixHover }
                SectionHeading { text: qsTr("Radios para ti") }
                RevealGate { id: mixGate; count: mixList.count }
                ListView {
                    id: mixList
                    objectName: "home_mixList"
                    x: -Theme.space.sm
                    width: parent.width + 2 * Theme.library.cardBleed
                    height: Theme.library.cardWidth + Theme.size.trackRow
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    model: Backend.Library.forYou
                    delegate: MediaCard {
                        id: mixCard
                        required property int index
                        transform: Translate { id: mixCardRise }
                        Reveal { id: mixCardReveal; item: mixCard; shift: mixCardRise; gate: mixGate; index: mixCard.index }
                        Component.onCompleted: mixCardReveal.begin()
                        ListView.onReused: mixCardReveal.settle()
                        required property string mixId
                        required property string name
                        required property string imageUrl
                        // MediaCard already has a subtitle property; bind the role explicitly.
                        required property var model
                        width: Theme.library.cardWidth
                        title: name === "" ? qsTr("Radio para ti") : name
                        subtitle: model.subtitle === "" ? qsTr("Inspirada en lo que escuchas") : qsTr("Radio · %1").arg(model.subtitle)
                        artSource: imageUrl
                        playable: true
                        onPlayRequested: Backend.Library.playMix(mixId)
                        onActiveFocusChanged: { if (activeFocus) viewport.revealCard(mixList, index) }
                        onClicked: Backend.Library.playMix(mixId)
                    }
                }
                ShelfArrows { pageViewport: viewport; list: mixList; hovered: mixHover.hovered }
            }
            // S9/N5: hidden while there is nothing to show (local history or Spotify).
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: recentList.count > 0
                HoverHandler { id: recentHover }
                SectionHeading { text: qsTr("Escuchado recientemente") }
                RevealGate { id: recentGate; count: recentList.count }
                ListView {
                    id: recentList
                    objectName: "home_recentList"
                    width: parent.width
                    height: Theme.size.playerBar
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    model: Backend.Library.recent
                    // R3 (P2-14): a click plays the song (or opens what it is) through Library.openLink, as
                    // in Spotify; «Añadir a la cola» lives in its context menu (right click, Menu key,
                    // Shift+F10), built on first use.
                    delegate: LibraryButton {
                        id: recentCard
                        objectName: "homeRecent" + index
                        required property int index
                        transform: Translate { id: recentRise }
                        Reveal { id: recentReveal; item: recentCard; shift: recentRise; gate: recentGate; index: recentCard.index }
                        Component.onCompleted: recentReveal.begin()
                        ListView.onReused: recentReveal.settle()
                        onActiveFocusChanged: { if (activeFocus) viewport.revealCard(recentList, index) }
                        required property string title
                        required property string artist
                        required property string uri
                        required property string imageUrl
                        readonly property string uriKind: uri.split(":")[1] || ""
                        width: Theme.size.coverMax
                        height: Theme.size.playerBar
                        text: title
                        Accessible.name: qsTr("%1, %2").arg(title).arg(artist)
                        Accessible.description: uriKind === "track" || uriKind === "episode" ? qsTr("Reproducir") : qsTr("Abrir")
                        background: Rectangle { color: recentCard.live ? Theme.surface3 : Theme.surface2; radius: Theme.radius.lg; border.width: recentCard.visualFocus ? Theme.stroke.focus : 0; border.color: Theme.focusRing }
                        contentItem: Row {
                            spacing: Theme.space.md
                            LibraryArtwork { width: Theme.size.trackRow; height: width; source: recentCard.imageUrl; anchors.verticalCenter: parent.verticalCenter }
                            Column {
                                width: recentCard.width - Theme.size.trackRow - Theme.space.md - 2 * Theme.space.lg
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: Theme.space.xs
                                LibraryLabel { width: parent.width; text: recentCard.title; font.weight: Theme.type.semibold }
                                LibraryLabel { width: parent.width; text: recentCard.artist; color: Theme.textSecondary; font.pixelSize: Theme.type.caption }
                            }
                        }
                        onClicked: Backend.Library.openLink(uri)
                        function queue() {
                            if (uriKind === "album" || uriKind === "playlist") Backend.Actions.addContextToQueue(uri)
                            else Backend.Library.addToQueue(uri)
                            page.queueRequested()
                        }
                        function openMenu(x, y) {
                            recentMenu.active = true
                            recentMenu.item.popup(recentCard, x, y)
                        }
                        TapHandler {
                            acceptedButtons: Qt.RightButton
                            onTapped: point => recentCard.openMenu(point.position.x, point.position.y)
                        }
                        Keys.onMenuPressed: openMenu(width / 2, height / 2)
                        Keys.onPressed: event => {
                            if (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier)) {
                                openMenu(width / 2, height / 2)
                                event.accepted = true
                            }
                        }
                        Loader {
                            id: recentMenu
                            active: false
                            sourceComponent: LibraryMenu {
                                objectName: "homeRecentMenu"
                                seedUri: recentCard.uri
                                Action { text: qsTr("Añadir a la cola"); onTriggered: recentCard.queue() }
                            }
                        }
                    }
                }
                ShelfArrows { pageViewport: viewport; list: recentList; hovered: recentHover.hovered; centerY: recentList.height / 2; step: Theme.size.coverMax }
            }
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: playlistList.count > 0
                HoverHandler { id: playlistHover }
                SectionHeading { text: qsTr("Tus playlists") }
                RevealGate { id: playlistGate; count: playlistList.count }
                ListView {
                    id: playlistList
                    objectName: "home_playlistList"
                    // Shifted by the card inset so the art lines up with the section heading.
                    x: -Theme.space.sm
                    width: parent.width + 2 * Theme.library.cardBleed
                    height: Theme.library.cardWidth + Theme.size.trackRow
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    model: Backend.Library.playlists
                    delegate: MediaCard {
                        id: playlistCard
                        required property int index
                        transform: Translate { id: playlistCardRise }
                        Reveal { id: playlistCardReveal; item: playlistCard; shift: playlistCardRise; gate: playlistGate; index: playlistCard.index }
                        Component.onCompleted: playlistCardReveal.begin()
                        ListView.onReused: playlistCardReveal.settle()
                        onActiveFocusChanged: { if (activeFocus) viewport.revealCard(playlistList, index) }
                        objectName: index === 0 ? "homeFirstPlaylistCard" : ""
                        required property string playlistId
                        required property string name
                        required property string owner
                        required property string imageUrl
                        width: Theme.library.cardWidth
                        artSource: imageUrl
                        title: name
                        subtitle: owner
                        uri: "spotify:playlist:" + playlistId
                        owned: owner !== "" && owner === Backend.Account.displayName
                        onClicked: page.navigate("playlist", playlistId, owner)
                    }
                }
                ShelfArrows { pageViewport: viewport; list: playlistList; hovered: playlistHover.hovered }
            }
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: artistList.count > 0
                HoverHandler { id: artistHover }
                SectionHeading { text: qsTr("Tus artistas más escuchados") }
                RevealGate { id: artistGate; count: artistList.count }
                ListView {
                    id: artistList
                    objectName: "home_artistList"
                    // Shifted by the card inset so the art lines up with the section heading.
                    x: -Theme.space.sm
                    width: parent.width + 2 * Theme.library.cardBleed
                    height: Theme.library.cardWidth + Theme.size.trackRow
                    orientation: ListView.Horizontal
                    spacing: Theme.space.md
                    reuseItems: true
                    clip: true
                    model: Backend.Library.topArtists
                    delegate: MediaCard {
                        id: artistCard
                        required property int index
                        transform: Translate { id: artistCardRise }
                        Reveal { id: artistCardReveal; item: artistCard; shift: artistCardRise; gate: artistGate; index: artistCard.index }
                        Component.onCompleted: artistCardReveal.begin()
                        ListView.onReused: artistCardReveal.settle()
                        onActiveFocusChanged: { if (activeFocus) viewport.revealCard(artistList, index) }
                        required property string artistId
                        required property string name
                        required property string imageUrl
                        width: Theme.library.cardWidth
                        artSource: imageUrl
                        title: name
                        subtitle: qsTr("Artista")
                        portrait: true
                        uri: "spotify:artist:" + artistId
                        onClicked: page.navigate("artist", artistId, "")
                    }
                }
                ShelfArrows { pageViewport: viewport; list: artistList; hovered: artistHover.hovered }
            }
            // H1: "Tus canciones más escuchadas" (last weeks), first rows.
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: topList.count > 0
                property bool expanded: false
                readonly property int shown: expanded ? topList.count : Math.min(topList.count, Theme.home.topTracksShown)
                SectionHeading { text: qsTr("Tus canciones más escuchadas") }
                RevealGate { id: topGate; count: topList.count }
                TextMetrics { id: topNumber; text: "00"; font.family: Theme.family; font.pixelSize: Theme.type.body; font.features: Theme.tabularFeatures }
                ListView {
                    id: topList
                    width: parent.width
                    height: parent.shown * Theme.size.trackRow
                    interactive: false
                    clip: true
                    reuseItems: true
                    model: Backend.Library.topTracks
                    delegate: TrackRow {
                        id: topRow
                        transform: Translate { id: topRise }
                        Reveal { id: topReveal; item: topRow; shift: topRise; gate: topGate; index: topRow.index }
                        Component.onCompleted: topReveal.begin()
                        ListView.onReused: topReveal.settle()
                        width: topList.width - Theme.space.sm
                        visible: index < topList.parent.shown
                        numberWidth: Math.ceil(topNumber.advanceWidth)
                        titleWidth: Math.max(0, width - 2 * Theme.trackList.rowInset - numberWidth - Theme.trackList.rowArt
                                    - Theme.trackList.trailingWidth - 5 * Theme.space.md)
                        albumWidth: 0
                        addedWidth: 0
                        playAction: i => Backend.Library.playTopTrack(i)
                    }
                }
                LibraryButton {
                    visible: topList.count > Theme.home.topTracksShown
                    text: parent.expanded ? qsTr("Ver menos") : qsTr("Ver más canciones")
                    onClicked: parent.expanded = !parent.expanded
                }
            }
            Column {
                width: parent.width
                spacing: Theme.space.md
                visible: recList.count > 0
                property bool expanded: false
                readonly property int shown: expanded ? recList.count : Math.min(recList.count, Theme.home.topTracksShown)
                SectionHeading { text: qsTr("Recomendado para ti") }
                RevealGate { id: recGate; count: recList.count }
                TextMetrics { id: recNumber; text: "00"; font.family: Theme.family; font.pixelSize: Theme.type.body; font.features: Theme.tabularFeatures }
                ListView {
                    id: recList
                    width: parent.width
                    height: parent.shown * Theme.size.trackRow
                    interactive: false
                    clip: true
                    reuseItems: true
                    model: Backend.Library.recommendedTracks
                    delegate: TrackRow {
                        id: recRow
                        transform: Translate { id: recRise }
                        Reveal { id: recReveal; item: recRow; shift: recRise; gate: recGate; index: recRow.index }
                        Component.onCompleted: recReveal.begin()
                        ListView.onReused: recReveal.settle()
                        width: recList.width - Theme.space.sm
                        visible: index < recList.parent.shown
                        numberWidth: Math.ceil(recNumber.advanceWidth)
                        titleWidth: Math.max(0, width - 2 * Theme.trackList.rowInset - numberWidth - Theme.trackList.rowArt
                                    - Theme.trackList.trailingWidth - 5 * Theme.space.md)
                        albumWidth: 0
                        addedWidth: 0
                        playAction: i => Backend.Library.playRecommendedTrack(i)
                    }
                }
                LibraryButton {
                    visible: recList.count > Theme.home.topTracksShown
                    text: parent.expanded ? qsTr("Ver menos") : qsTr("Ver más canciones")
                    onClicked: parent.expanded = !parent.expanded
                }
            }
        }
    }
}
