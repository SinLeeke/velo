//! F4: search and sort inside the open song list, over the rows already loaded. Pure functions
//! plus the row store the library engine keeps; the engine runs them on the tokio runtime and
//! replaces the QML model once per change.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use serde_json::Value;
use velo_domain::Track;

/// "Clasificar por".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// The list's own order ("Orden personalizado").
    #[default]
    Custom,
    Title,
    Artist,
    Album,
    /// "Agregado recientemente".
    Added,
    Duration,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "custom" => Self::Custom,
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "added" => Self::Added,
            "duration" => Self::Duration,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Added => "added",
            Self::Duration => "duration",
        }
    }

    /// Direction when the key is first chosen: newest first for "Agregado recientemente".
    pub fn default_descending(self) -> bool {
        self == Self::Added
    }
}

/// How one list is shown; remembered per list for the session (memory only).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewSpec {
    pub query: String,
    pub sort: SortKey,
    pub descending: bool,
    /// "Ver como: Compacta" (QML only; does not change the rows).
    pub compact: bool,
}

impl ViewSpec {
    /// Rows are shown in a different order than the list's own.
    pub fn reordered(&self) -> bool {
        self.sort != SortKey::Custom || self.descending
    }

    pub fn filtered(&self) -> bool {
        !fold(&self.query).trim().is_empty()
    }

    /// The model shows the loaded rows as they are (no view mapping).
    pub fn is_identity(&self) -> bool {
        !self.reordered() && !self.filtered()
    }

    /// "Clasificar por": choosing the current key again flips the direction.
    pub fn choose_sort(&mut self, key: SortKey) {
        if key == self.sort {
            self.descending = !self.descending;
        } else {
            self.sort = key;
            self.descending = key.default_descending();
        }
    }
}

/// One loaded row: its model row (kept up to date as thumbnails arrive) and its folded keys.
#[derive(Debug, Clone)]
pub struct Entry {
    pub row: Value,
    pub uri: String,
    /// Remote cover URL still to shrink (kept to re-request it after a removal).
    pub image: Option<String>,
    title: String,
    artist: String,
    album: String,
    added_at: i64,
    duration_ms: u64,
}

impl Entry {
    pub fn new(track: &Track, row: Value, image: Option<String>) -> Self {
        Self {
            row,
            uri: track.uri.clone(),
            image,
            title: fold(&track.title),
            artist: fold(&track.artist_line()),
            album: fold(&track.album),
            added_at: track.added_at.unwrap_or(0),
            duration_ms: track.duration_ms,
        }
    }

    /// Entry from a model row (search results arrive as rows, not domain tracks).
    pub fn from_row(row: Value) -> Self {
        let text = |key: &str| row[key].as_str().unwrap_or_default().to_string();
        Self {
            uri: text("uri"),
            image: None,
            title: fold(&text("title")),
            artist: fold(&text("artist")),
            album: fold(&text("album")),
            added_at: 0,
            duration_ms: row["durationMs"].as_u64().unwrap_or(0),
            row,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    fn matches(&self, terms: &[&str]) -> bool {
        terms.iter().all(|term| {
            self.title.contains(term) || self.artist.contains(term) || self.album.contains(term)
        })
    }
}

/// Indexes of `entries` to show, in display order, for `spec`. Ties keep the list's order.
pub fn compute_view(entries: &[Entry], spec: &ViewSpec) -> Vec<usize> {
    let query = fold(&spec.query);
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut view: Vec<usize> = (0..entries.len())
        .filter(|&i| terms.is_empty() || entries[i].matches(&terms))
        .collect();
    let by = |a: &Entry, b: &Entry| -> Ordering {
        match spec.sort {
            SortKey::Custom => Ordering::Equal,
            SortKey::Title => a.title.cmp(&b.title),
            SortKey::Artist => a.artist.cmp(&b.artist).then_with(|| a.album.cmp(&b.album)),
            SortKey::Album => a.album.cmp(&b.album),
            SortKey::Added => a.added_at.cmp(&b.added_at),
            SortKey::Duration => a.duration_ms.cmp(&b.duration_ms),
        }
    };
    view.sort_by(|&a, &b| {
        let order = by(&entries[a], &entries[b]);
        let order = if spec.descending {
            order.reverse()
        } else {
            order
        };
        // Reversed custom order is the list read from the end.
        order.then_with(|| {
            if spec.descending && spec.sort == SortKey::Custom {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        })
    });
    view
}

/// Lowercase without accents, for searching and sorting ("Canción" == "cancion").
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if ('\u{0300}'..='\u{036f}').contains(&c) {
            continue;
        }
        out.push(match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ą' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' | 'ő' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ñ' | 'ń' | 'ň' => 'n',
            'ç' | 'ć' | 'č' => 'c',
            'š' | 'ś' => 's',
            'ž' | 'ź' | 'ż' => 'z',
            'ł' => 'l',
            'ř' => 'r',
            'ť' => 't',
            'ď' => 'd',
            other => other,
        });
    }
    out
}

/// The loaded rows of the open list and how they are shown.
#[derive(Debug, Default)]
pub struct TrackStore {
    /// Bumped whenever the rows are replaced or removed; late thumbnails of an older
    /// generation are dropped.
    pub generation: u64,
    pub entries: Vec<Entry>,
    /// Display order (indexes into `entries`); `None` while the spec is the identity.
    pub view: Option<Vec<usize>>,
    /// Inverse of `view`: entry → displayed row.
    pos: Vec<Option<usize>>,
    pub spec: ViewSpec,
    /// F5 is transient, independent of the remembered search/sort view.
    pub active_tag: String,
    pub tags: HashMap<String, Vec<String>>,
}

impl TrackStore {
    pub fn clear(&mut self) {
        self.generation += 1;
        self.entries.clear();
        self.view = None;
        self.pos.clear();
        self.active_tag.clear();
        self.tags.clear();
    }

    /// Displayed row of entry `i`, if shown.
    pub fn shown_row(&self, i: usize) -> Option<usize> {
        match &self.view {
            None => (i < self.entries.len()).then_some(i),
            Some(_) => self.pos.get(i).copied().flatten(),
        }
    }

    /// Entry behind displayed row `row`.
    pub fn entry_of(&self, row: usize) -> Option<usize> {
        match &self.view {
            None => (row < self.entries.len()).then_some(row),
            Some(view) => view.get(row).copied(),
        }
    }

    /// URIs in display order.
    pub fn shown_uris(&self) -> Vec<String> {
        match &self.view {
            None => self.entries.iter().map(|e| e.uri.clone()).collect(),
            Some(view) => view.iter().map(|&i| self.entries[i].uri.clone()).collect(),
        }
    }

    /// Recomputes the view for `spec`; returns the rows to show (in display order).
    pub fn apply(&mut self) -> Vec<Value> {
        if self.spec.is_identity() && self.active_tag.is_empty() {
            self.view = None;
            self.pos.clear();
            return self.entries.iter().map(|e| e.row.clone()).collect();
        }
        let mut view = compute_view(&self.entries, &self.spec);
        if !self.active_tag.is_empty() {
            view.retain(|&i| {
                self.tags
                    .get(&self.entries[i].uri)
                    .is_some_and(|tags| tags.contains(&self.active_tag))
            });
        }
        self.pos = vec![None; self.entries.len()];
        for (row, &i) in view.iter().enumerate() {
            self.pos[i] = Some(row);
        }
        let rows = view.iter().map(|&i| self.entries[i].row.clone()).collect();
        self.view = Some(view);
        rows
    }

    pub fn duration_ms(&self) -> u64 {
        self.entries.iter().map(Entry::duration_ms).sum()
    }

    pub fn toggle_tag(&mut self, tag: &str) {
        self.active_tag = if self.active_tag == tag {
            String::new()
        } else {
            tag.into()
        };
    }

    /// Count a label once per row, even when both genre and mood carry it.
    /// Duplicated songs count as occurrences in the list; ties sort by label.
    pub fn top_tags(&self) -> Vec<String> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for entry in &self.entries {
            if let Some(tags) = self.tags.get(&entry.uri) {
                let mut seen = HashSet::new();
                for tag in tags.iter().filter(|t| !t.is_empty()) {
                    if seen.insert(tag) {
                        *counts.entry(tag).or_default() += 1;
                    }
                }
            }
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
        counts
            .into_iter()
            .take(12)
            .map(|(tag, _)| tag.into())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use velo_domain::ArtistRef;

    #[test]
    fn f5_tags_count_rows_deduplicate_labels_sort_frequency_and_cap_at_twelve() {
        let mut st = TrackStore {
            entries: sample(),
            ..Default::default()
        };
        st.entries.push(st.entries[0].clone());
        st.tags.insert(
            st.entries[0].uri.clone(),
            vec!["rock".into(), "rock".into(), "calm".into()],
        );
        st.tags
            .insert(st.entries[1].uri.clone(), vec!["rock".into()]);
        st.tags.insert(
            st.entries[2].uri.clone(),
            (0..15).map(|i| format!("tag{i:02}")).collect(),
        );
        let tags = st.top_tags();
        assert_eq!(tags.len(), 12);
        assert_eq!(&tags[..3], &["rock", "calm", "tag00"]);
        assert_eq!(tags[11], "tag09");
        assert!(TrackStore::default().top_tags().is_empty());
    }

    #[test]
    fn f5_filter_combines_search_sort_toggles_off_and_clear_resets_tags() {
        let mut st = TrackStore {
            entries: sample(),
            ..Default::default()
        };
        for i in [0, 2] {
            st.tags
                .insert(st.entries[i].uri.clone(), vec!["calm".into()]);
        }
        st.toggle_tag("calm");
        st.spec.choose_sort(SortKey::Title);
        assert_eq!(
            st.apply()
                .iter()
                .map(|r| r["title"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Bruma", "Canción del mar"]
        );
        assert_eq!(st.entry_of(0), Some(2));
        assert_eq!(st.shown_row(1), None);
        st.spec.query = "atlas".into();
        assert_eq!(st.apply().len(), 1);
        st.toggle_tag("calm");
        assert!(st.active_tag.is_empty());
        st.spec = ViewSpec::default();
        assert_eq!(st.apply().len(), 3);
        st.toggle_tag("calm");
        st.toggle_tag("");
        assert!(st.active_tag.is_empty());
        st.toggle_tag("calm");
        st.clear();
        assert!(st.active_tag.is_empty() && st.tags.is_empty() && st.top_tags().is_empty());
    }

    fn entry(title: &str, artist: &str, album: &str, added: i64, ms: u64) -> Entry {
        let t = Track {
            id: None,
            uri: format!("spotify:track:{title}"),
            title: title.into(),
            artists: vec![ArtistRef {
                id: None,
                name: artist.into(),
            }],
            album: album.into(),
            album_id: None,
            duration_ms: ms,
            explicit: false,
            images: vec![],
            added_at: Some(added),
        };
        Entry::new(&t, json!({ "title": title }), None)
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry("Canción del mar", "Lumen", "Atlas", 30, 200),
            entry("arena", "Íria Sol", "Bosque", 10, 300),
            entry("Bruma", "lumen", "Agua", 20, 100),
        ]
    }

    fn spec(query: &str, sort: SortKey, descending: bool) -> ViewSpec {
        ViewSpec {
            query: query.into(),
            sort,
            descending,
            compact: false,
        }
    }

    #[test]
    fn folding_ignores_case_and_accents() {
        assert_eq!(fold("Canción ÁRBOL Ñandú"), "cancion arbol nandu");
        assert_eq!(fold("Cancio\u{0301}n"), "cancion");
        assert_eq!(fold("Straße"), "straße");
    }

    #[test]
    fn search_matches_title_artist_or_album_without_case_or_accents() {
        let e = sample();
        assert_eq!(
            compute_view(&e, &spec("CANCION", SortKey::Custom, false)),
            [0]
        );
        assert_eq!(compute_view(&e, &spec("iria", SortKey::Custom, false)), [1]);
        assert_eq!(compute_view(&e, &spec("agua", SortKey::Custom, false)), [2]);
        assert_eq!(
            compute_view(&e, &spec("lumen", SortKey::Custom, false)),
            [0, 2]
        );
        // Every word must match somewhere.
        assert_eq!(
            compute_view(&e, &spec("lumen atlas", SortKey::Custom, false)),
            [0]
        );
        assert!(compute_view(&e, &spec("nada", SortKey::Custom, false)).is_empty());
        assert_eq!(
            compute_view(&e, &spec("  ", SortKey::Custom, false)),
            [0, 1, 2]
        );
    }

    #[test]
    fn every_sort_key_and_direction() {
        let e = sample();
        let order = |k, d| compute_view(&e, &spec("", k, d));
        assert_eq!(order(SortKey::Title, false), [1, 2, 0]);
        assert_eq!(order(SortKey::Title, true), [0, 2, 1]);
        // Same artist (folded): ties broken by album, then the list's order.
        assert_eq!(order(SortKey::Artist, false), [1, 2, 0]);
        assert_eq!(order(SortKey::Album, false), [2, 0, 1]);
        assert_eq!(order(SortKey::Added, true), [0, 2, 1]);
        assert_eq!(order(SortKey::Added, false), [1, 2, 0]);
        assert_eq!(order(SortKey::Duration, false), [2, 0, 1]);
        assert_eq!(order(SortKey::Custom, false), [0, 1, 2]);
        assert_eq!(order(SortKey::Custom, true), [2, 1, 0]);
    }

    #[test]
    fn choosing_the_same_key_flips_and_a_new_key_takes_its_default() {
        let mut s = ViewSpec::default();
        assert!(s.is_identity());
        s.choose_sort(SortKey::Added);
        assert_eq!((s.sort, s.descending), (SortKey::Added, true));
        s.choose_sort(SortKey::Added);
        assert!(!s.descending);
        s.choose_sort(SortKey::Title);
        assert_eq!((s.sort, s.descending), (SortKey::Title, false));
        s.choose_sort(SortKey::Custom);
        assert!(s.is_identity());
        s.choose_sort(SortKey::Custom);
        assert!(s.reordered());
        for k in ["custom", "title", "artist", "album", "added", "duration"] {
            assert_eq!(SortKey::parse(k).unwrap().as_str(), k);
        }
        assert_eq!(SortKey::parse("x"), None);
    }

    #[test]
    fn store_maps_rows_both_ways_and_back_to_identity() {
        let mut store = TrackStore {
            entries: sample(),
            ..TrackStore::default()
        };
        assert_eq!(store.apply().len(), 3);
        assert_eq!(store.shown_row(2), Some(2));
        store.spec = spec("lumen", SortKey::Title, true);
        let rows = store.apply();
        assert_eq!(
            rows,
            [
                json!({"title": "Canción del mar"}),
                json!({"title": "Bruma"})
            ]
        );
        assert_eq!(store.entry_of(1), Some(2));
        assert_eq!(store.shown_row(1), None);
        assert_eq!(store.shown_row(0), Some(0));
        assert_eq!(
            store.shown_uris(),
            ["spotify:track:Canción del mar", "spotify:track:Bruma"]
        );
        store.spec = ViewSpec::default();
        store.apply();
        assert_eq!(store.entry_of(1), Some(1));
        assert_eq!(store.duration_ms(), 600);
    }
}
