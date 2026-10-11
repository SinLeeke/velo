import QtQuick
import QtTest
import Velo.Backend as Backend
import "../../theme"
import "../../components/library"

// Lavado de cara, grupo B: pulsar una canción se ve en la fila (presión, número→reproducir,
// pendiente, acento), tarjetas con presión, y los botones que faltaban (Aleatorio de artista,
// pódcast, perfil, Reproducir en el menú de canción, Mixear en «Tus me gusta»).
TestCase {
    id: test
    name: "FeelB"
    when: windowShown
    visible: true
    width: 900
    height: 700

    Component {
        id: rowComponent
        TrackRow {
            width: 700
            index: 2
            uri: "spotify:track:feelb"
            title: "Canción"
            artist: "Artista"
            album: "Álbum"
            durationMs: 180000
            imageUrl: ""
            liked: false
            addedLabel: ""
            artistId: "a"
            albumId: "b"
            titleWidth: 300
        }
    }
    Component { id: cardComponent; MediaCard { width: 180; title: "Lista"; subtitle: "Playlist"; uri: "spotify:playlist:p1" } }
    Component { id: tracksComponent; TrackListPage { width: test.width; height: test.height; playlistId: "feelb-list" } }
    Component { id: artistComponent; ArtistPage { width: test.width; height: test.height; artistId: "feelb-artist" } }
    Component { id: podcastComponent; PodcastPage { width: test.width; height: test.height; showId: "feelb-show"; playAction: i => { test.played = i } } }
    Component { id: profileComponent; ProfilePage { width: test.width; height: test.height } }
    Component { id: menuComponent; TrackMenu {} }
    property int played: -1

    function init() {
        Theme.reduceMotion = true   // the end state is what is checked: instant
        Theme.performanceMode = false
        Backend.Player.canControl = true
        Backend.Player.trackUri = "spotify:track:other"
        Backend.Player.isPlaying = true
        Backend.Player.contextUri = ""
        Backend.Library.lastPlayedIndex = -1
        Backend.Library.shuffleCurrentCount = 0
        Backend.Library.mixCurrent = false
        Backend.Library.currentKind = "playlist"
        Backend.Library.currentListable = true
        played = -1
    }
    function cleanup() {
        Theme.reduceMotion = false
        Backend.Player.canControl = true
        Backend.Player.trackUri = "spotify:track:mock"
        Backend.Player.contextUri = ""
    }

    // ---- TrackRow

    function test_row_pending_until_the_song_changes() {
        const row = createTemporaryObject(rowComponent, test)
        verify(row !== null)
        verify(!row.pending && !row.lit)
        const title = findChild(row, "trackTitleLit")
        compare(title.opacity, 0)
        row.play()
        verify(row.pending && row.lit)
        compare(Backend.Library.lastPlayedIndex, 2)
        tryCompare(title, "opacity", 1)
        Backend.Player.trackUri = row.uri
        verify(!row.pending)
        verify(row.playingHere && row.lit)
        compare(title.opacity, 1)
    }

    function test_row_pending_clears_on_error_and_does_not_start_on_the_playing_row() {
        const row = createTemporaryObject(rowComponent, test)
        row.play()
        verify(row.pending)
        Backend.Player.error("x")
        verify(!row.pending)
        Backend.Player.trackUri = row.uri
        row.play()
        verify(!row.pending)
    }

    function test_row_number_turns_into_a_play_glyph_on_hover() {
        const row = createTemporaryObject(rowComponent, test)
        const glyph = findChild(row, "trackPlayGlyph")
        compare(glyph.opacity, 0)
        mouseMove(row, 200, row.height / 2)
        tryCompare(glyph, "opacity", 1)
        mouseMove(test, 5, 600)
        tryCompare(glyph, "opacity", 0)
    }

    function test_row_number_click_plays_without_double_click() {
        const row = createTemporaryObject(rowComponent, test)
        const hit = findChild(row, "trackNumberHit")
        mouseMove(row, 200, row.height / 2)
        const p = hit.mapToItem(row, hit.width / 2, hit.height / 2)
        mouseClick(row, p.x, p.y)
        compare(Backend.Library.lastPlayedIndex, 2)
        verify(row.pending)
    }

    function test_row_press_layer_shows_while_pressed() {
        const row = createTemporaryObject(rowComponent, test)
        const layer = findChild(row, "trackPressLayer")
        compare(layer.opacity, 0)
        mousePress(row, 400, row.height / 2)
        tryCompare(layer, "opacity", 1)
        mouseRelease(row, 400, row.height / 2)
        tryCompare(layer, "opacity", 0)
    }

    function test_row_effects_off_in_performance_mode_still_correct() {
        Theme.performanceMode = true
        const row = createTemporaryObject(rowComponent, test)
        row.play()
        verify(row.pending)
        tryCompare(findChild(row, "trackTitleLit"), "opacity", 1)
    }

    // ---- MediaCard

    function test_card_presses_in_and_returns() {
        const card = createTemporaryObject(cardComponent, test)
        compare(card.scale, 1)
        mousePress(card, 40, 40)
        tryCompare(card, "scale", Theme.feel.pressScaleWide)
        mouseRelease(card, 40, 40)
        tryCompare(card, "scale", 1)
    }

    function test_card_of_what_plays_takes_the_accent() {
        const card = createTemporaryObject(cardComponent, test)
        const title = findChild(card, "cardTitleLit")
        compare(title.opacity, 0)
        Backend.Player.contextUri = "spotify:playlist:p1"
        verify(card.playingHere)
        tryCompare(title, "opacity", 1)
        Backend.Player.contextUri = "spotify:playlist:other"
        tryCompare(title, "opacity", 0)
    }

    // ---- Botones que faltaban

    function test_song_menu_starts_with_play() {
        const menu = createTemporaryObject(menuComponent, test)
        compare(menu.itemAt(0).text, qsTr("Reproducir"))
        const row = createTemporaryObject(rowComponent, test)
        menu.fill(row)
        menu.itemAt(0).action.trigger()
        compare(Backend.Library.lastPlayedIndex, 2)
    }

    function test_artist_has_a_shuffle_button_usable_cold() {
        Backend.Player.canControl = false
        const page = createTemporaryObject(artistComponent, test)
        const button = findChild(page, "artistShuffleButton")
        verify(button !== null)
        verify(findChild(page, "artistShuffleSwitch") === null)
        verify(button.enabled)   // Premium is enough: no active session needed
        mouseClick(button)
        compare(Backend.Library.shuffleCurrentCount, 1)
    }

    function test_liked_gets_mix_but_albums_do_not() {
        const page = createTemporaryObject(tracksComponent, test)
        const button = findChild(page, "mixButton")
        Backend.Library.currentKind = "liked"
        verify(button.visible)
        mouseClick(button)
        verify(Backend.Library.mixCurrent)
        Backend.Library.currentKind = "album"
        verify(!button.visible)
    }

    function test_podcast_header_plays_the_latest_episode_and_has_a_menu() {
        Backend.Library.openShow("feelb-show")
        Backend.Player.canControl = false
        const page = createTemporaryObject(podcastComponent, test)
        const play = findChild(page, "podcastPlayButton")
        verify(play !== null)
        tryVerify(() => page.listView.count > 0)
        verify(play.enabled)
        mouseClick(play)
        compare(test.played, 0)
        const more = findChild(page, "podcastMoreButton")
        verify(more !== null)
        mouseClick(more)
        const menu = findChild(page, "shareMenu")
        verify(menu !== null)
        tryVerify(() => menu.visible)
        verify(menu.link.indexOf("open.spotify.com/show/feelb-show") >= 0)
        menu.close()
    }

    function test_podcast_episode_play_is_usable_cold_and_menu_offers_the_queue() {
        Backend.Library.openShow("feelb-show")
        Backend.Player.canControl = false
        const page = createTemporaryObject(podcastComponent, test)
        tryVerify(() => page.listView.count > 0)
        tryVerify(() => page.listView.itemAtIndex(0) !== null)
        const episode = page.listView.itemAtIndex(0)
        verify(findChild(episode, "episodePlayButton").enabled)
        episode.openMenu(10, 10)
        let queue = null
        for (let i = 0; i < page.children.length; ++i) {}
        tryVerify(() => {
            const menus = page.data.filter(o => o && o.objectName === "shareMenu" && o.queueEntry)
            queue = menus.length ? menus[0] : null
            return queue !== null && queue.visible
        })
        compare(queue.itemAt(0).text, qsTr("Añadir a la cola"))
        queue.close()
    }

    function test_profile_has_a_more_button_with_copy_link() {
        Backend.Library.openProfile("iria")
        const page = createTemporaryObject(profileComponent, test)
        const more = findChild(page, "profileMoreButton")
        verify(more !== null)
        tryVerify(() => more.visible)
        mouseClick(more)
        const menu = findChild(page, "shareMenu")
        tryVerify(() => menu.visible)
        verify(menu.link.indexOf("open.spotify.com/user/") >= 0)
        menu.close()
    }

    function test_mock_exposes_the_new_player_and_library_api() {
        verify("contextUri" in Backend.Player && "contextTitle" in Backend.Player)
        verify(!Backend.Library.mixPlaying)
        Backend.Library.toggleMixPlaying()
        verify(Backend.Library.mixPlaying)
        Backend.Library.toggleMixPlaying()
        verify(!Backend.Library.mixPlaying)
    }
}
