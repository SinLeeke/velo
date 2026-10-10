import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../components/library"
import "../../theme"

TestCase {
    id: test
    name: "R4"
    visible: true
    width: 960
    height: 640
    when: windowShown
    property int playedIndex: -1
    property int plays: 0
    property int globalSpaces: 0
    property var snapshot: ({})
    readonly property url art: Qt.resolvedUrl("../../mock/assets/cover_teal.png")
    Item { id: host; width: 750; height: 450 }
    Shortcut { sequence: "Space"; onActivated: test.globalSpaces++ }
    Component { id: albumComponent; AlbumGridPage { width: host.width; height: host.height } }
    Component { id: artistGridComponent; ArtistGridPage { width: host.width; height: host.height } }
    Component { id: tracksComponent; TrackListPage { width: host.width; height: host.height; playlistId: "r4-list" } }
    Component { id: podcastComponent; PodcastPage { width: host.width; height: host.height; showId: "r4-show"; playAction: i => { test.playedIndex = i; test.plays++ } } }
    Component { id: profileComponent; ProfilePage { width: host.width; height: host.height } }
    Component { id: searchComponent; SearchPage { width: host.width; height: host.height } }
    Component { id: searchBoxComponent; SearchBox { width: 480 } }
    Component { id: creditsComponent; CreditsDialog {} }
    Component { id: trackMenuComponent; TrackMenu {} }
    Component { id: artistComponent; ArtistPage { width: host.width; height: host.height; artistId: "r4-artist" } }
    Component { id: mediaMenuComponent; MediaMenu { uri: "spotify:playlist:p1"; owned: true; mediaTitle: "R4" } }

    function rows(model) {
        const result = []
        for (let i = 0; i < model.count; ++i) result.push(Object.assign({}, model.get(i)))
        return result
    }
    function replace(model, values) { model.clear(); values.forEach(value => model.append(value)) }
    function initTestCase() {
        const names = ["tracks", "savedAlbums", "followedArtists", "searchResults", "searchArtists", "searchAlbums", "searchPlaylists", "searchBestResult", "searchSuggestions", "profilePlaylists", "profileTopArtists", "topTracks"]
        names.forEach(name => snapshot[name] = rows(Backend.Library[name]))
    }
    function init() {
        Theme.reduceMotion = true
        Backend.Player.canControl = true
        Backend.Library.loading = false
        Backend.Library.loadError = ""
        Backend.Library.hasMore = false
        Backend.Library.currentKind = "playlist"
        Backend.Library.currentListable = true
        Backend.Library.searchQuery = ""
        Backend.Library.searchError = ""
        Backend.Library.searchLoading = false
        Backend.Library.searchHasMore = false
        Backend.Library.searchType = "all"
        Backend.Library.cancelSearchSuggestions()
        playedIndex = -1
        plays = 0
        globalSpaces = 0
        host.width = 750
        host.height = 450
    }
    function cleanup() {
        Object.keys(snapshot).forEach(name => replace(Backend.Library[name], snapshot[name]))
        Backend.Library.loading = false
        Backend.Library.loadError = ""
        Backend.Library.searchError = ""
        Backend.Library.searchLoading = false
        Backend.Library.searchQuery = ""
        Backend.Library.cancelSearchSuggestions()
    }
    function track(index) {
        return { trackId: "r4-" + index, uri: "spotify:track:r4-" + index, title: "R4 " + index,
            artist: "Artista", album: "Álbum", durationMs: 180000, imageUrl: art.toString(),
            liked: false, artistId: "r4-a", albumId: "r4-al", addedLabel: "" }
    }
    function result(index) {
        return { kind: "artist", objectId: "r4-a" + index, uri: "spotify:artist:r4-a" + index,
            title: "Artista " + index, subtitle: "Artista", imageUrl: art.toString(), artistId: "", albumId: "", album: "", year: 2026, durationMs: 0 }
    }
    function assertTabs(view, lastIndex) {
        view.positionViewAtIndex(0, GridView.Contain)
        view.forceLayout()
        tryVerify(() => view.itemAtIndex(0) !== null)
        view.itemAtIndex(0).forceActiveFocus(Qt.TabFocusReason)
        for (let i = 1; i <= lastIndex; ++i) {
            keyClick(Qt.Key_Tab)
            tryCompare(view, "currentIndex", i)
            verify(view.itemAtIndex(i).activeFocus)
        }
        verify(view.contentY > view.originY || view.contentX > view.originX, "Tab desplazó la vista")
        keyClick(Qt.Key_Backtab)
        tryCompare(view, "currentIndex", lastIndex - 1)
    }
    function imageOf(item) {
        if (item.source !== undefined && item.source.toString() === art.toString() && item.asynchronous === true) return item
        const children = item.children || []
        for (let i = 0; i < children.length; ++i) { const found = imageOf(children[i]); if (found) return found }
        return null
    }
    function actionFor(menu, label) {
        for (let i = 0; i < menu.count; ++i) {
            const item = menu.itemAt(i)
            if (item && item.text === label) return item.action
        }
        return null
    }
    function test_album_grid_tab_and_decode_steps() {
        Backend.Library.savedAlbums.clear()
        for (let i = 0; i < 80; ++i) Backend.Library.savedAlbums.append({ albumId: "r4-al" + i, name: "Álbum " + i, artist: "Artista", imageUrl: art.toString(), year: 2026 })
        const page = createTemporaryObject(albumComponent, host)
        verify(page !== null)
        const grid = findChild(page, "albumGrid")
        assertTabs(grid, 35)
        const image = imageOf(grid.itemAtIndex(35))
        verify(image !== null)
        const size = image.sourceSize.width
        host.width++
        wait(0)
        compare(image.sourceSize.width, size, "Un píxel de ventana no vuelve a decodificar")
    }
    function test_artist_grid_tab_outside_view() {
        Backend.Library.followedArtists.clear()
        for (let i = 0; i < 80; ++i) Backend.Library.followedArtists.append({ artistId: "r4-a" + i, name: "Artista " + i, imageUrl: "" })
        const page = createTemporaryObject(artistGridComponent, host)
        verify(page !== null)
        assertTabs(findChild(page, "artistGrid"), 35)
    }
    function test_search_grid_and_shelf_tab_outside_view() {
        replace(Backend.Library.searchResults, Array.from({ length: 80 }, (_, i) => result(i)))
        Backend.Library.searchQuery = "R4"
        Backend.Library.searchType = "artist"
        const page = createTemporaryObject(searchComponent, host)
        verify(page !== null)
        assertTabs(findChild(page, "searchGrid"), 35)
        Backend.Library.searchType = "all"
        replace(Backend.Library.searchArtists, Array.from({ length: 80 }, (_, i) => result(i)))
        wait(0)
        const shelf = findChild(page, "searchShelf_artist")
        verify(shelf !== null)
        assertTabs(shelf, 15)
    }
    function test_load_empty_error_and_retry() {
        Backend.Library.tracks.clear()
        Backend.Library.loading = true
        const page = createTemporaryObject(tracksComponent, host)
        verify(page !== null)
        const status = findChild(page, "listFooterStatus")
        verify(!status.visible, "La primera carga no anuncia vacío")
        Backend.Player.error("No se pudo conectar. Revisa tu conexión.")
        Backend.Library.loading = false
        tryCompare(page, "failed", true)
        compare(status.text, "No se pudo conectar. Revisa tu conexión.")
        const retry = findChild(page, "listRetryButton")
        verify(retry.visible)
        retry.clicked()
        compare(page.localLoadError, "")
        Backend.Library.tracks.clear()
        Backend.Library.currentKind = "liked"
        tryVerify(() => status.text.indexOf("corazón") >= 0)
    }
    function test_grid_states_data() { return [{ tag: "albums", component: albumComponent, model: "savedAlbums", prefix: "albumGrid" }, { tag: "artists", component: artistGridComponent, model: "followedArtists", prefix: "artistGrid" }] }
    function test_grid_states(data) {
        Backend.Library[data.model].clear()
        Backend.Library.loading = true
        const page = createTemporaryObject(data.component, host)
        verify(page !== null)
        const status = findChild(page, data.prefix + "Status")
        verify(status.text.indexOf("Cargando") === 0)
        Backend.Library.loadError = "No se pudo cargar."
        Backend.Library.loading = false
        compare(status.text, "No se pudo cargar.")
        verify(findChild(page, data.prefix + "Retry").visible)
        Backend.Library.loadError = ""
        verify(status.text.indexOf("aparecerán") >= 0)
    }
    function test_track_menu_shared_heart_and_space() {
        replace(Backend.Library.tracks, Array.from({ length: 60 }, (_, i) => track(i)))
        const page = createTemporaryObject(tracksComponent, host)
        verify(page !== null)
        const view = page.listView
        view.positionViewAtIndex(0, ListView.Contain)
        view.forceLayout()
        tryVerify(() => view.itemAtIndex(0) !== null && view.itemAtIndex(1) !== null)
        const first = view.itemAtIndex(0), second = view.itemAtIndex(1)
        compare(first.sharedMenu, second.sharedMenu)
        compare(first.sharedMenu, page.trackMenu)
        first.openMenu()
        tryCompare(page.trackMenu, "opened", true)
        compare(page.trackMenu.uri, first.uri)
        compare(page.trackMenu.shareLink, "https://open.spotify.com/track/r4-0")
        page.trackMenu.close()
        tryCompare(page.trackMenu, "opened", false)
        first.forceActiveFocus(Qt.TabFocusReason)
        const like = findChild(first, "trackLikeButton")
        like.clicked()
        compare(first.liked, true)
        like.clicked()
        compare(first.liked, false)
        first.playAction = i => { test.playedIndex = i; test.plays++ }
        keyClick(Qt.Key_Space)
        compare(playedIndex, 0)
        compare(globalSpaces, 0)
        for (let i = 0; i < 40; ++i) keyClick(Qt.Key_Tab)
        compare(view.currentIndex, 20, "Tab crea filas más allá del cacheBuffer")
    }
    function test_podcast_enter_and_tab() {
        Backend.Library.currentKind = "show"
        replace(Backend.Library.tracks, Array.from({ length: 60 }, (_, i) => track(i)))
        const page = createTemporaryObject(podcastComponent, host)
        verify(page !== null)
        const view = page.listView
        view.positionViewAtIndex(0, ListView.Contain)
        view.forceLayout()
        tryVerify(() => view.itemAtIndex(0) !== null)
        view.itemAtIndex(0).forceActiveFocus(Qt.TabFocusReason)
        keyClick(Qt.Key_Return)
        compare(playedIndex, 0)
        compare(plays, 1)
        keyClick(Qt.Key_Space)
        compare(plays, 2)
        compare(globalSpaces, 0)
        for (let i = 0; i < 40; ++i) keyClick(Qt.Key_Tab)
        compare(view.currentIndex, 20)
        compare(page.durationText(3600000), "1 h 0 min")
    }
    function test_profile_hides_empty_sections_and_heading() {
        Backend.Library.profileStatus = "ready"
        Backend.Library.profilePlaylistsAvailable = true
        Backend.Library.profilePlaylists.clear()
        const page = createTemporaryObject(profileComponent, host)
        verify(page !== null)
        verify(!findChild(page, "profilePlaylistsSection").visible)
        compare(findChild(page, "profileName").Accessible.role, Accessible.Heading)
    }
    function test_credits_empty_hidden_and_retry() {
        const dialog = createTemporaryObject(creditsComponent, host)
        verify(dialog !== null)
        dialog.openFor("spotify:track:r4-empty")
        tryCompare(dialog, "opened", true)
        dialog.accept(JSON.stringify({ title: "R4", contributors: [] }))
        tryCompare(dialog, "opened", false)
        const menu = createTemporaryObject(trackMenuComponent, host)
        menu.fill({ uri: "spotify:track:r4-empty", liked: false, artistId: "", albumId: "", playlistId: "", ownPlaylist: false })
        verify(menu.creditsKnownEmpty, "La siguiente apertura oculta Ver créditos")
        dialog.openFor("spotify:track:r4-credits")
        dialog.accept("{")
        compare(dialog.status, "failed")
        verify(findChild(dialog, "creditsRetryButton").visible)
        findChild(dialog, "creditsRetryButton").clicked()
        compare(dialog.status, "ready")
        compare(dialog.roleLabel("producer", "Producer"), "Producción")
        dialog.close()
    }
    function test_media_menu_owned_actions_and_share() {
        const menu = createTemporaryObject(mediaMenuComponent, host)
        verify(menu !== null)
        menu.popup(host, 0, 0)
        tryCompare(menu, "opened", true)
        compare(menu.shareLink, "https://open.spotify.com/playlist/p1")
        verify(menu.detailsKnown)
        menu.close()
        actionFor(menu, "Editar detalles").trigger()
        const edit = findChild(menu, "playlistEditDialog")
        verify(edit !== null)
        tryCompare(edit, "opened", true)
        compare(findChild(edit, "playlistNameField").text, menu.mediaTitle)
        compare(findChild(edit, "playlistDescriptionField").text, menu.description)
        edit.close()
        tryCompare(edit, "opened", false)
        actionFor(menu, "Eliminar").trigger()
        const cancel = findChild(menu, "confirmCancelButton")
        verify(cancel !== null)
        verify(cancel.activeFocus, "Eliminar pone el foco en Cancelar")
        compare(menu.deleteUri, "spotify:playlist:p1")
        menu.uri = "spotify:playlist:p2"
        compare(menu.deleteUri, "spotify:playlist:p1", "La confirmación conserva la identidad original")
        cancel.clicked()
    }
    function test_suggestion_save_reflects_membership() {
        Backend.Actions.setSaved("spotify:track:r4-suggest", true)
        // T3: the suggestions live in the top bar's SearchBox (SearchPage has no field now).
        const page = createTemporaryObject(searchBoxComponent, host)
        verify(page !== null)
        const field = findChild(page, "librarySearchField")
        field.text = "R4"
        field.forceActiveFocus(Qt.TabFocusReason)
        Backend.Library.searchSuggestions.append({ kind: "track", objectId: "r4-suggest", uri: "spotify:track:r4-suggest", title: "R4", subtitle: "Artista", imageUrl: "" })
        Backend.Library.searchSuggestionQuery = "R4"
        page.suggestIndex = 0
        tryCompare(page.suggestionPopup, "opened", true)
        const list = findChild(page, "searchSuggestionList")
        list.forceLayout()
        tryVerify(() => list.itemAtIndex(0) !== null)
        const button = findChild(list.itemAtIndex(0), "suggestionSaveButton")
        tryCompare(button, "active", true)
        compare(button.text, "Quitar de Tus me gusta")
        waitForRendering(button)
        verify(button.enabled)
        mouseClick(button)
        tryCompare(button, "active", false)
        compare(button.text, "Guardar en Tus me gusta")
        verify(field.activeFocus, "Guardar conserva el campo de búsqueda")
    }
    function test_artist_has_shared_menu_and_heading() {
        Backend.Library.artistStatus = "ready"
        const page = createTemporaryObject(artistComponent, host)
        verify(page !== null)
        compare(findChild(page, "artistTitle").Accessible.role, Accessible.Heading)
        const list = findChild(page, "artistPopularList")
        list.forceLayout()
        tryVerify(() => list.itemAtIndex(0) !== null)
        verify(list.itemAtIndex(0).sharedMenu !== null)
    }
}
