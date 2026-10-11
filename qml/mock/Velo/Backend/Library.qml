pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Library (docs/CONTRATO.md, including the wave-2 additions).
QtObject {
    id: library
    // Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
    // H1: datos ficticios; imágenes vacías/locales, sin red ni Timer.
    property string madeForYouStatus: "ready"
    property ListModel madeForYou: ListModel {
        ListElement { playlistId: "made1"; name: "Discover Weekly"; description: "Selección de ejemplo"; imageUrl: ""; owner: "Spotify" }
        ListElement { playlistId: "made2"; name: "Daily Mix 1"; description: "Mix de ejemplo"; imageUrl: ""; owner: "Spotify" }
    }
    property ListModel quickAccess: ListModel {}
    property ListModel topTracks: ListModel {
        ListElement { trackId: "t1"; uri: "spotify:track:t1"; title: "Marea de cobalto"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 261000; imageUrl: ""; liked: true; artistId: "a1"; albumId: "al1"; addedLabel: "" }
        ListElement { trackId: "t2"; uri: "spotify:track:t2"; title: "Insomne"; artist: "Bosque Bajo"; album: "Canciones y demos"; durationMs: 210000; imageUrl: ""; liked: false; artistId: "a2"; albumId: "al2"; addedLabel: "" }
    }
    function playTopTrack(index) {}
    property ListModel recommendedTracks: ListModel {
        ListElement { trackId: "r1"; uri: "spotify:track:r1"; title: "Faro sin costa"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 198000; imageUrl: ""; liked: false; artistId: "a1"; albumId: "al1"; addedLabel: "" }
        ListElement { trackId: "r2"; uri: "spotify:track:r2"; title: "Sal en la ventana"; artist: "Bosque Bajo"; album: "Canciones y demos"; durationMs: 224000; imageUrl: ""; liked: true; artistId: "a2"; albumId: "al2"; addedLabel: "" }
    }
    function playRecommendedTrack(index) {}
    // S9/N5: the eight "Acceso rápido" tiles, in Spotify's order (liked is not forced first).
    function _homeQuickAccess() {
        quickAccess.clear()
        for (let i = 0; i < Math.min(3, playlists.count); ++i) {
            const p = playlists.get(i)
            quickAccess.append({ kind: "playlist", objectId: p.playlistId, name: p.name, imageUrl: p.imageUrl, uri: "spotify:playlist:" + p.playlistId })
            if (i === 0) quickAccess.append({ kind: "liked", objectId: "", name: "", imageUrl: "", uri: "spotify:user:juan:collection" })
        }
        quickAccess.append({ kind: "artist", objectId: "a1", name: "Lumen Fiel", imageUrl: "", uri: "spotify:artist:a1" })
        quickAccess.append({ kind: "album", objectId: "al2", name: "Canciones y demos 2019–2023", imageUrl: "", uri: "spotify:album:al2" })
        quickAccess.append({ kind: "playlist", objectId: "made1", name: "Discover Weekly", imageUrl: "", uri: "spotify:playlist:made1" })
        quickAccess.append({ kind: "album", objectId: "al1", name: "Atlas de agua", imageUrl: "", uri: "spotify:album:al1" })
    }
    signal navigateRequested(string kind, string id)
    function openLink(text) {
        const entries = text.trim().split(/\s+/)
        const links = entries.map(function (entry) {
            let normalized = entry.replace(/^https?:\/\/open\.spotify\.com\/(intl-[a-z-]+\/)?/i, "spotify:")
                                  .replace(/[?#].*$/, "").replace(/\/$/, "")
            normalized = normalized.replace(/^spotify:user:[^:]+:playlist:/i, "spotify:playlist:")
            normalized = normalized.replace(/^spotify:user\/[^/]+\/playlist\//i, "spotify:playlist:")
            const match = /^spotify:(track|album|playlist|artist)[:\/]([a-z0-9]{1,64})$/i.exec(normalized)
            return match ? {kind: match[1].toLowerCase(), id: match[2]} : null
        })
        if (links.some(function (link) { return !link })) return
        if (links.length > 1) {
            if (links.every(function (link) { return link.kind === "track" }))
                links.forEach(function (link) { addToQueue("spotify:track:" + link.id) })
        } else if (links[0].kind === "track") {
            Player.title = qsTr("Canción del enlace")
            Player.isPlaying = true
        } else navigateRequested(links[0].kind, links[0].id)
    }
    // F6: datos ficticios para preview, no respuestas de Spotify.
    property string artistStatus: "none"
    property string artistBiography: qsTr("Biografía de ejemplo para previsualizar el perfil de artista.")
    property string artistDiscographyType: "all"
    property bool artistIncomplete: false
    property ListModel artistPopular: ListModel {
        ListElement { trackId: "t1"; uri: "spotify:track:t1"; title: "Marea de cobalto"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 261000; imageUrl: ""; liked: false; artistId: "a1"; albumId: "al1"; addedLabel: "" }
    }
    property ListModel artistRelated: ListModel {
        ListElement { artistId: "a2"; name: "Bosque Bajo"; imageUrl: "" }
    }
    property ListModel artistDiscography: ListModel {}
    property ListModel artistAppearsOn: ListModel {
        ListElement { albumId: "al3"; name: "Voces de invierno"; artist: "Varios artistas"; imageUrl: ""; year: 2025 }
    }
    property ListModel artistPhotos: ListModel {
        ListElement { imageUrl: "" }
    }
    function setArtistDiscographyType(kind) {
        if (["all", "album", "single", "compilation"].indexOf(kind) < 0) return
        artistDiscographyType = kind
        artistDiscography.clear()
        const releases = [
            { albumId: "al1", name: "Atlas de agua", artist: "Lumen Fiel", imageUrl: "", year: 2024, albumType: "album" },
            { albumId: "al4", name: "Marea", artist: "Lumen Fiel", imageUrl: "", year: 2025, albumType: "single" },
            { albumId: "al6", name: "Horizonte", artist: "Lumen Fiel", imageUrl: "", year: 2025, albumType: "ep" },
            { albumId: "al5", name: "Antología", artist: "Lumen Fiel", imageUrl: "", year: 2025, albumType: "compilation" }
        ]
        for (let r of releases) if (kind === "all" || kind === r.albumType || (kind === "single" && r.albumType === "ep")) artistDiscography.append(r)
    }
    function playArtistPopular(index) {}
    // F1/F2: datos ficticios; el mock no usa red ni timers.
    property string searchQuery: ""
    property string searchSuggestionQuery: ""
    property string searchType: "all"
    property bool searchLoading: false
    property bool searchSuggesting: false
    property bool searchHasMore: false
    property string searchError: ""
    property string searchSuggestionError: ""
    property string followedArtistId: ""
    property bool artistFollowed: false
    property bool artistFollowedKnown: false
    property bool artistFollowBusy: false
    property ListModel searchSuggestions: ListModel {}
    property ListModel searchResults: ListModel {}
    property ListModel searchTracks: ListModel {}
    property ListModel searchArtists: ListModel {}
    property ListModel searchPlaylists: ListModel {}
    property ListModel searchAlbums: ListModel {}
    property ListModel searchBestResult: ListModel {}
    function mockSearchRows() {
        return [
            { kind: "track", objectId: "t1", uri: "spotify:track:t1", title: "Marea de cobalto", subtitle: "Lumen Fiel", imageUrl: "", artistId: "a1", albumId: "al1", album: "Atlas de agua", year: 2024, durationMs: 261000, following: false, followingKnown: false, followingBusy: false },
            { kind: "artist", objectId: "a1", uri: "spotify:artist:a1", title: "Lumen Fiel", subtitle: "", imageUrl: "", artistId: "a1", albumId: "", album: "", year: 0, durationMs: 0, following: artistFollowed, followingKnown: artistFollowedKnown, followingBusy: false },
            { kind: "playlist", objectId: "p1", uri: "spotify:playlist:p1", title: "Para concentrarse", subtitle: "Juan", imageUrl: "", artistId: "", albumId: "", album: "", year: 0, durationMs: 0, following: false, followingKnown: false, followingBusy: false },
            { kind: "album", objectId: "al1", uri: "spotify:album:al1", title: "Atlas de agua", subtitle: "Lumen Fiel", imageUrl: "", artistId: "", albumId: "al1", album: "Atlas de agua", year: 2024, durationMs: 0, following: false, followingKnown: false, followingBusy: false }
        ]
    }
    function suggestSearch(query) {
        searchSuggestionQuery = query.trim(); searchSuggestions.clear()
        if (searchSuggestionQuery) for (let row of mockSearchRows()) searchSuggestions.append(row)
    }
    function cancelSearchSuggestions() { searchSuggestionQuery = ""; searchSuggestions.clear() }
    function setSearchType(kind) {
        if (["all", "track", "artist", "playlist", "album"].indexOf(kind) < 0) return
        searchType = kind; searchResults.clear()
        if (searchQuery) for (let row of mockSearchRows()) if (kind === "all" || kind === row.kind) searchResults.append(row)
    }
    function fetchMoreSearch() {}
    function playSearchResult(index) {}
    function playBestSearchResult() {}
    function inspectArtistFollowed(id) { followedArtistId = id; artistFollowedKnown = true }
    function setArtistFollowed(id, followed) {
        followedArtistId = id; artistFollowedKnown = true; artistFollowed = followed
        for (let model of [searchSuggestions, searchResults, searchArtists, searchBestResult])
            for (let i = 0; i < model.count; ++i) if (model.get(i).kind === "artist" && model.get(i).objectId === id) {
                model.setProperty(i, "following", followed); model.setProperty(i, "followingKnown", true)
            }
    }
    property bool forYouAvailable: true
    property ListModel forYou: ListModel {}
    // Six illustrative radios; this worktree's covers live in qml/mock/assets.
    Component.onCompleted: {
        _homeQuickAccess()
        _rebuildSidebar()
        const names = [qsTr("Lumen Fiel"), qsTr("Bosque Bajo"), qsTr("Marea de cobalto"), qsTr("Iria Sol"), qsTr("Faro sin costa"), qsTr("Insomne")]
        const subtitles = [qsTr("Atlas de agua"), qsTr("Canciones y demos"), qsTr("Lumen Fiel"), qsTr("Bosque Bajo"), qsTr("Lumen Fiel"), qsTr("Bosque Bajo, Iria Sol")]
        for (let i = 0; i < names.length; ++i)
            forYou.append({ mixId: "mix" + i, name: names[i], subtitle: subtitles[i], imageUrl: Qt.resolvedUrl(i % 2 ? "../../assets/cover_green.png" : "../../assets/cover_teal.png").toString() })
    }
    function playMix(mixId) {}
    function saveRadioAsPlaylist(seedUri) {}

    property ListModel playlists: ListModel {
        ListElement { playlistId: "p1"; name: "Para concentrarse"; owner: "Juan"; imageUrl: ""; trackCount: 48; pinned: true; owned: true; description: "Música tranquila para trabajar" }
        ListElement { playlistId: "p2"; name: "Domingo lento"; owner: "Juan"; imageUrl: ""; trackCount: 23; pinned: false; owned: true; description: "" }
        ListElement { playlistId: "p3"; name: "Carretera de noche"; owner: "Iria"; imageUrl: ""; trackCount: 61; pinned: false; owned: false; description: "" }
    }
    property ListModel tracks: ListModel {
        ListElement { trackId: "t1"; uri: "spotify:track:t1"; title: "Marea de cobalto"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 261000; imageUrl: ""; liked: true; artistId: "a1"; albumId: "al1"; addedLabel: "hace 2 días" }
        ListElement { trackId: "t2"; uri: "spotify:track:t2"; title: "Insomne"; artist: "Bosque Bajo, Iria Sol"; album: "Canciones y demos 2019–2023"; durationMs: 210000; imageUrl: ""; liked: true; artistId: "a2"; albumId: "al2"; addedLabel: "12 sept 2025" }
        ListElement { trackId: "t3"; uri: "spotify:track:t3"; title: "Faro sin costa"; artist: "Lumen Fiel"; album: "Atlas de agua"; durationMs: 198000; imageUrl: ""; liked: false; artistId: "a1"; albumId: "al1"; addedLabel: "3 ene 2024" }
    }
    property ListModel recent: ListModel {
        ListElement { trackId: "t2"; uri: "spotify:track:t2"; title: "Insomne"; artist: "Bosque Bajo, Iria Sol"; album: "Canciones y demos 2019–2023"; durationMs: 210000; imageUrl: ""; liked: true; artistId: "a2"; albumId: "al2"; addedLabel: "" }
    }
    property ListModel topArtists: ListModel {
        ListElement { artistId: "a1"; name: "Lumen Fiel"; imageUrl: "" }
        ListElement { artistId: "a2"; name: "Bosque Bajo"; imageUrl: "" }
    }
    property ListModel followedArtists: ListModel {
        ListElement { artistId: "a2"; name: "Bosque Bajo"; imageUrl: "" }
    }
    property ListModel savedAlbums: ListModel {
        ListElement { albumId: "al1"; name: "Atlas de agua"; artist: "Lumen Fiel"; imageUrl: ""; year: 2024 }
    }
    property ListModel albums: ListModel {
        ListElement { albumId: "al2"; name: "Canciones y demos 2019–2023"; artist: "Bosque Bajo"; imageUrl: ""; year: 2023 }
    }

    property string currentTitle: "Para concentrarse"
    property string currentSubtitle: "Juan"
    property url currentImageUrl: ""
    property string currentKind: "playlist"
    property bool currentListable: true
    property bool loading: false
    // R4: user-facing message when the open list / grid (playlist, album, Tus me gusta, saved
    // albums, followed artists, artist, podcast) or its next page failed to load; "" otherwise.
    // Cleared when a new open*/fetchMore starts. The page offers "Reintentar".
    property string loadError: ""
    property bool hasMore: false
    // F3: header totals of playlist / "Tus me gusta".
    property int currentTrackCount: 48
    property real currentDurationMs: 9060000
    property bool currentDurationExact: true
    // F4: search and sort inside the open list (the real filtering runs in Rust on tokio; this
    // mock does it in JS over a snapshot of `tracks`, only for previews).
    property string filterQuery: ""
    property string sortKey: "custom"
    property bool sortDescending: false
    property bool compactView: false
    property bool mixCurrent: false
    // Mixear: el contexto que SUENA está marcado (PREVIEW: alterna con toggleMixPlaying).
    property bool mixPlaying: false
    property int mixPlayingToggles: 0
    property int lastPlayedIndex: -1        // PREVIEW: último playIndex pedido
    property bool viewBusy: false
    property var _allRows: null
    function _fold(text) { return String(text).toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "") }
    function _applyView() {
        if (_allRows === null) {
            _allRows = []
            for (let i = 0; i < tracks.count; ++i) _allRows.push(Object.assign({}, tracks.get(i)))
        }
        const identity = filterQuery.trim() === "" && sortKey === "custom" && !sortDescending && activeTag === ""
        let rows = _allRows.map((row, i) => ({ row: row, i: i }))
        const terms = _fold(filterQuery).split(/\s+/).filter(t => t !== "")
        rows = rows.filter(r => terms.every(t => [r.row.title, r.row.artist, r.row.album].some(f => _fold(f).indexOf(t) >= 0)))
        if (activeTag !== "") rows = rows.filter(r => _rowTags(r.row).indexOf(activeTag) >= 0)
        const field = { title: "title", artist: "artist", album: "album", duration: "durationMs", added: null }[sortKey]
        rows.sort((a, b) => {
            let order = 0
            if (sortKey === "added") order = a.i - b.i   // mock rows are newest first
            else if (field) order = field === "durationMs" ? a.row[field] - b.row[field] : _fold(a.row[field]).localeCompare(_fold(b.row[field]))
            if (sortKey === "added") order = -order
            if (sortDescending) order = -order
            return order !== 0 ? order : (sortDescending && sortKey === "custom" ? b.i - a.i : a.i - b.i)
        })
        tracks.clear()
        for (const r of rows) tracks.append(r.row)
        if (identity) _allRows = null
    }
    function setFilter(q) { filterQuery = q; _applyView() }
    function setSort(key) {
        if (key === sortKey) sortDescending = !sortDescending
        else { sortKey = key; sortDescending = key === "added" }
        _applyView()
    }
    function setCompact(on) {
        compactView = on
        if (_compactKey !== "") _compactOverrides[_compactKey] = on
    }
    // F5: genre/mood chips of the open list. Rust derives the real tags; this mock gives each
    // fixture artist a few and counts them over the whole list (most frequent first, max 12).
    // Empty while loading or without data: the chip row is then not shown at all.
    property var trackTags: ["Calma", "Indie", "Lo-fi", "Para concentrarse", "Folk", "Nocturna", "Acústico", "Melancólica"]
    property string activeTag: ""
    readonly property var _tagsByArtist: ({
        a1: ["Indie", "Calma", "Lo-fi", "Para concentrarse"],
        a2: ["Folk", "Calma", "Nocturna", "Acústico", "Melancólica"]
    })
    function _rowTags(row) { return _tagsByArtist[row.artistId] || [] }
    function _refreshTags() {
        if (!currentListable) { trackTags = []; return }
        const rows = []
        if (_allRows !== null) for (const row of _allRows) rows.push(row)
        else for (let i = 0; i < tracks.count; ++i) rows.push(tracks.get(i))
        const counts = {}
        for (const row of rows) for (const tag of _rowTags(row)) counts[tag] = (counts[tag] || 0) + 1
        trackTags = Object.keys(counts).sort((a, b) => counts[b] - counts[a] || a.localeCompare(b)).slice(0, 12)
    }
    function filterTag(tag) { activeTag = tag; _applyView() }

    function _openList() {
        if (activeTag !== "") filterTag("")
        _restoreTracks()
        _refreshTags()
        // S9: Settings.compactTracks is the initial view; Library.setCompact keeps a per-list
        // override for the session (contract S9/N2).
        const key = currentKind + ":" + currentOwnerId + ":" + currentTitle
        _compactKey = key
        compactView = _compactOverrides[key] !== undefined ? _compactOverrides[key] : Settings.compactTracks === true
    }
    property string _compactKey: ""
    property var _compactOverrides: ({})
    function openPlaylist(id) {
        currentKind = "playlist"
        currentOwnerId = ""
        for (let i = 0; i < playlists.count; ++i)
            if (playlists.get(i).playlistId === id) currentOwnerId = playlists.get(i).owned ? "juan" : String(playlists.get(i).owner).toLowerCase()
        _openList()
    }
    function openLiked() { currentKind = "liked"; currentOwnerId = ""; _openList() }
    // U4: profile page (fixture data; "me"/"juan" = the signed-in account, "x" = unavailable).
    property string currentOwnerId: ""
    property string profileStatus: "none"
    property string profileUserId: ""
    property string profileName: ""
    property url profileImageUrl: ""
    property int profileFollowers: -1
    property int profilePlaylistCount: -1
    property bool profileIsMe: false
    property bool profilePlaylistsAvailable: false
    property ListModel profileTopArtists: ListModel {}
    property ListModel profilePlaylists: ListModel {}
    function openProfile(userId) {
        currentKind = "profile"
        currentOwnerId = ""
        profileTopArtists.clear()
        profilePlaylists.clear()
        const me = userId === "me" || userId === "juan"
        profileUserId = me ? "juan" : userId
        profileIsMe = me
        profileName = me ? Account.displayName : userId === "x" ? "" : userId.charAt(0).toUpperCase() + userId.slice(1)
        profileImageUrl = me ? Account.avatarUrl : ""
        profileFollowers = me ? 12 : userId === "iria" ? 1 : -1
        profilePlaylistsAvailable = userId !== "x"
        if (userId === "x") { profilePlaylistCount = -1; profileStatus = "unavailable"; return }
        for (let i = 0; i < playlists.count; ++i) {
            const p = playlists.get(i)
            if (p.owned === me && (me || String(p.owner).toLowerCase() === userId))
                profilePlaylists.append({ playlistId: p.playlistId, name: p.name, owner: p.owner, imageUrl: p.imageUrl, trackCount: p.trackCount, pinned: false, owned: me, description: p.description })
        }
        if (me) for (let j = 0; j < topArtists.count; ++j) profileTopArtists.append(topArtists.get(j))
        profilePlaylistCount = profilePlaylists.count
        profileStatus = "ready"
    }
    function search(q) {
        currentKind = "search"; searchQuery = q.trim(); cancelSearchSuggestions()
        for (let model of [searchTracks, searchArtists, searchPlaylists, searchAlbums, searchBestResult]) model.clear()
        if (searchQuery) {
            const rows = mockSearchRows()
            searchTracks.append(rows[0]); searchArtists.append(rows[1]); searchPlaylists.append(rows[2]); searchAlbums.append(rows[3]); searchBestResult.append(rows[1])
        }
        setSearchType("all")
    }
    function fetchMore() {}
    function playIndex(i) { lastPlayedIndex = i }
    function playCurrent() {}
    property int shuffleCurrentCount: 0     // PREVIEW
    function shuffleCurrent() { shuffleCurrentCount += 1 }
    function toggleMixCurrent() { mixCurrent = !mixCurrent }
    function toggleMixPlaying() { mixPlaying = !mixPlaying; mixPlayingToggles += 1 }
    function togglePin(playlistId) {
        for (let i = 0; i < playlists.count; ++i)
            if (playlists.get(i).playlistId === playlistId)
                playlists.setProperty(i, "pinned", !playlists.get(i).pinned)
    }
    function addToQueue(uri) {}
    // R3 (contract request): play an album / playlist / artist from its first track without
    // opening it (hover play disc of the cards). The mock only records it.
    property string lastPlayedContext: ""
    function playContext(uri) { lastPlayedContext = uri }
    function loadHome() { currentKind = "home"; _homeQuickAccess() }
    function openAlbum(id) { currentKind = "album"; _restoreTracks() }
    function openArtist(id) { currentKind = "artist"; artistStatus = "ready"; setArtistDiscographyType("all") }
    function openSavedAlbums() { currentKind = "albums" }
    function openFollowedArtists() { currentKind = "artists" }
    // ---- S9/N3: unified sidebar (fixture data; the real model is filtered/sorted in Rust).
    // Roles: kind, objectId, uri, name, creator, imageUrl, pinned, owned, addedAtMs, lastPlayedAtMs.
    property ListModel sidebarItems: ListModel {}
    property string sidebarFilter: "all"        // all | playlist | album | artist | show
    property string sidebarQuery: ""
    property string sidebarStatus: "ready"      // none | loading | ready | partial | unavailable
    property string sidebarError: ""
    property string showsStatus: "ready"
    // S9/N5: where Library.recent comes from (local history, Spotify, nothing).
    property string recentSource: "local"
    readonly property real _day: 86400000
    readonly property real _now: 1791500000000
    // Playlist dates (added, last played); -1 = unknown (sorted last).
    readonly property var _playlistDates: ({
        p1: [_now - 40 * _day, _now - 1 * _day], p2: [_now - 90 * _day, -1], p3: [_now - 10 * _day, _now - 3 * _day]
    })
    property var _sidebarOthers: [
        { kind: "album", objectId: "al1", name: "Atlas de agua", creator: "Lumen Fiel", imageUrl: "", owned: false, addedAtMs: _now - 5 * _day, lastPlayedAtMs: _now - 2 * _day },
        { kind: "album", objectId: "al2", name: "Canciones y demos 2019–2023", creator: "Bosque Bajo", imageUrl: "", owned: false, addedAtMs: _now - 200 * _day, lastPlayedAtMs: -1 },
        { kind: "artist", objectId: "a1", name: "Lumen Fiel", creator: "", imageUrl: "", owned: false, addedAtMs: _now - 30 * _day, lastPlayedAtMs: _now - 4 * _day },
        { kind: "artist", objectId: "a2", name: "Bosque Bajo", creator: "", imageUrl: "", owned: false, addedAtMs: _now - 300 * _day, lastPlayedAtMs: -1 },
        { kind: "show", objectId: "s1", name: "Historias de sobremesa", creator: "Radio Faro", imageUrl: "", owned: false, addedAtMs: _now - 12 * _day, lastPlayedAtMs: _now - 6 * _day }
    ]
    property var _pinnedUris: ({})
    function _sidebarSort() { return Settings.librarySort !== undefined ? Settings.librarySort : "recent_played" }
    function _rebuildSidebar() {
        let rows = []
        for (let i = 0; i < playlists.count; ++i) {
            const p = playlists.get(i)
            const d = _playlistDates[p.playlistId] || [-1, -1]
            rows.push({ kind: "playlist", objectId: p.playlistId, uri: "spotify:playlist:" + p.playlistId, name: p.name, creator: p.owner,
                        imageUrl: p.imageUrl, pinned: p.pinned, owned: p.owned, addedAtMs: d[0], lastPlayedAtMs: d[1] })
        }
        for (const o of _sidebarOthers) {
            const uri = "spotify:" + o.kind + ":" + o.objectId
            rows.push(Object.assign({}, o, { uri: uri, pinned: _pinnedUris[uri] === true }))
        }
        const terms = _fold(sidebarQuery).split(/\s+/).filter(t => t !== "")
        rows = rows.filter(r => (sidebarFilter === "all" || r.kind === sidebarFilter)
                                && (r.kind !== "show" || showsStatus !== "unavailable")
                                && terms.every(t => _fold(r.name + " " + r.creator).indexOf(t) >= 0))
        const sort = _sidebarSort()
        const date = r => sort === "recent_added" ? r.addedAtMs : r.lastPlayedAtMs
        rows.sort((a, b) => {
            if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
            let order = 0
            if (sort === "alphabetical") order = _fold(a.name).localeCompare(_fold(b.name))
            else if (sort === "creator") order = _fold(a.creator).localeCompare(_fold(b.creator))
            else {
                const da = date(a), db = date(b)
                order = da < 0 && db < 0 ? 0 : da < 0 ? 1 : db < 0 ? -1 : db - da
            }
            return order !== 0 ? order : _fold(a.name).localeCompare(_fold(b.name)) || a.uri.localeCompare(b.uri)
        })
        sidebarItems.clear()
        for (const r of rows) sidebarItems.append(r)
    }
    property Connections _playlistWatch: Connections {
        target: library.playlists
        function onCountChanged() { Qt.callLater(library._rebuildSidebar) }
        function onDataChanged() { Qt.callLater(library._rebuildSidebar) }
    }
    property Connections _sortWatch: Connections {
        target: Settings
        ignoreUnknownSignals: true
        function onLibrarySortChanged() { Qt.callLater(library._rebuildSidebar) }
    }
    function setSidebarFilter(kind) {
        sidebarFilter = ["all", "playlist", "album", "artist", "show"].indexOf(kind) >= 0 ? kind : "all"
        _rebuildSidebar()
    }
    function searchLibrary(text) { sidebarQuery = text.trim(); _rebuildSidebar() }
    function refreshSidebar() { _rebuildSidebar() }
    function toggleLibraryPin(uri) {
        if (uri.startsWith("spotify:playlist:")) { togglePin(uri.slice("spotify:playlist:".length)); return }
        const pins = Object.assign({}, _pinnedUris)
        pins[uri] = !(pins[uri] === true)
        _pinnedUris = pins
        _rebuildSidebar()
    }
    // S9/N3: podcast page; episodes use the `tracks` roles (artist = show, addedLabel = date).
    property var _savedTracks: null
    function _restoreTracks() {
        if (_savedTracks === null) return
        tracks.clear()
        for (const row of _savedTracks) tracks.append(row)
        _savedTracks = null
    }
    function openShow(id) {
        if (_savedTracks === null) {
            _savedTracks = []
            for (let i = 0; i < tracks.count; ++i) _savedTracks.push(Object.assign({}, tracks.get(i)))
        }
        currentKind = "show"
        currentOwnerId = ""
        currentTitle = "Historias de sobremesa"
        currentSubtitle = "Radio Faro"
        currentImageUrl = ""
        currentListable = true
        hasMore = false
        tracks.clear()
        const titles = ["El faro que nadie apagaba", "Recetas de la abuela, con ruido de fondo", "Cartas que llegaron tarde"]
        const dates = ["2 oct 2026", "25 sept 2026", "18 sept 2026"]
        for (let i = 0; i < titles.length; ++i)
            tracks.append({ trackId: "e" + i, uri: "spotify:episode:e" + i, title: titles[i], artist: "Historias de sobremesa", album: "Radio Faro",
                            durationMs: 1800000 + i * 420000, imageUrl: "", liked: false, artistId: "", albumId: "", addedLabel: dates[i] })
    }
    function createPlaylist(name) {
        playlists.append({ playlistId: "p" + (playlists.count + 1), name: name, owner: "Juan", imageUrl: "", trackCount: 0, pinned: false, owned: true, description: "" })
    }
    // M1b: "Editar detalles" (the real one waits for Spotify; the mock applies it at once).
    function updatePlaylistDetails(playlistId, name, description) {
        name = name.trim()
        if (!name) return
        for (let i = 0; i < playlists.count; ++i)
            if (playlists.get(i).playlistId === playlistId) {
                playlists.setProperty(i, "name", name)
                playlists.setProperty(i, "description", description.trim())
            }
    }
    function addToPlaylist(playlistId, uri) {}
    function removeFromPlaylist(playlistId, uri) {}
}
