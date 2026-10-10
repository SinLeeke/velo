use crate::{atomic_write, error, hash, project_dirs};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use velo_domain::{LyricLine, Lyrics, LyricsQuery, LyricsService, Result};

const NEGATIVE_TTL: u64 = 7 * 24 * 60 * 60;
const TRANSIENT_TTL: u64 = 10 * 60;
const USER_AGENT: &str = "Velo/0.1 (https://github.com/velo-app/velo)";

/// Parse LRC timestamps (centiseconds/milliseconds), global offset and timed pauses.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut offset = 0i64;
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((tag, remaining)) = after.split_once(']') else {
                break;
            };
            if let Some(value) = tag.strip_prefix("offset:") {
                if let Ok(value) = value.trim().parse::<i64>() {
                    offset = value;
                }
            } else if let Some(time) = timestamp(tag) {
                times.push(time);
            } else if !tag.split_once(':').is_some_and(|(key, _)| {
                matches!(
                    key,
                    "ar" | "ti" | "al" | "by" | "re" | "ve" | "length" | "au" | "id"
                )
            }) {
                // Bracketed lyric text such as [Chorus] is content, not metadata.
                break;
            }
            rest = remaining;
        }
        for time in times {
            lines.push(LyricLine {
                time_ms: time,
                text: rest.to_owned(),
            });
        }
    }
    for line in &mut lines {
        line.time_ms = if offset >= 0 {
            line.time_ms.saturating_add(offset as u64)
        } else {
            line.time_ms.saturating_sub(offset.unsigned_abs())
        };
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}
fn timestamp(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;
    if !minutes.bytes().all(|c| c.is_ascii_digit()) || minutes.is_empty() {
        return None;
    }
    let minutes = minutes.parse::<u64>().ok()?;
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if seconds.is_empty() || !seconds.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let seconds = seconds.parse::<u64>().ok()?;
    if seconds >= 60 || fraction.len() > 3 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let millis = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10u64.pow(3 - fraction.len() as u32)
    };
    minutes
        .checked_mul(60_000)?
        .checked_add(seconds * 1000)?
        .checked_add(millis)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}
impl Response {
    fn lyrics(self) -> Lyrics {
        Lyrics {
            synced: self
                .synced_lyrics
                .as_deref()
                .map(parse_lrc)
                .unwrap_or_default(),
            plain: self.plain_lyrics.filter(|s| !s.trim().is_empty()),
            instrumental: self.instrumental,
        }
    }
    fn has_synced(&self) -> bool {
        self.synced_lyrics
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
    }
    fn has_plain(&self) -> bool {
        self.plain_lyrics
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
    }
}
#[derive(Serialize, Deserialize)]
struct Entry {
    saved_at: u64,
    #[serde(default = "negative_ttl")]
    negative_ttl: u64,
    lyrics: Option<Lyrics>,
}
fn negative_ttl() -> u64 {
    NEGATIVE_TTL
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct LrcLibClient {
    root: PathBuf,
    base: String,
    client: crate::settings::proxy::HttpClient,
}
impl LrcLibClient {
    pub fn new() -> Result<Self> {
        Self::with_cache_dir(project_dirs()?.cache_dir())
    }
    pub fn with_cache_dir(root: impl Into<PathBuf>) -> Result<Self> {
        Self::with_base_url(root, "https://lrclib.net")
    }
    pub fn with_base_url(root: impl Into<PathBuf>, base: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            root: root.into(),
            base: base.as_ref().trim_end_matches('/').into(),
            client: crate::settings::proxy::HttpClient::new(Duration::from_secs(10))?,
        })
    }
    fn cache_path(&self, q: &LyricsQuery) -> PathBuf {
        let key = format!("{}|{}|{}|{}", q.artist, q.title, q.album, q.duration_s);
        self.root
            .join("lyrics")
            .join(format!("{}.json", hash(&key)))
    }
    async fn fetch(&self, q: &LyricsQuery) -> Result<(Option<Response>, u64)> {
        if q.duration_s != 0 {
            let mut url = url::Url::parse(&format!("{}/api/get", self.base)).map_err(error)?;
            url.query_pairs_mut().extend_pairs([
                ("track_name", q.title.as_str()),
                ("artist_name", q.artist.as_str()),
                ("album_name", q.album.as_str()),
                ("duration", &q.duration_s.to_string()),
            ]);
            let response = self
                .client
                .get(url)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .send()
                .await
                .map_err(error)?;
            if transient(response.status()) {
                return Ok((None, TRANSIENT_TTL));
            }
            if !matches!(
                response.status(),
                reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::BAD_REQUEST
            ) {
                let bytes = response
                    .error_for_status()
                    .map_err(error)?
                    .bytes()
                    .await
                    .map_err(error)?;
                return tokio::task::spawn_blocking(move || {
                    serde_json::from_slice(&bytes)
                        .map(|r| (Some(r), NEGATIVE_TTL))
                        .map_err(error)
                })
                .await
                .map_err(error)?;
            }
        }
        let mut url = url::Url::parse(&format!("{}/api/search", self.base)).map_err(error)?;
        url.query_pairs_mut()
            .extend_pairs([("track_name", &q.title), ("artist_name", &q.artist)]);
        let response = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(error)?;
        if transient(response.status()) {
            return Ok((None, TRANSIENT_TTL));
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok((None, NEGATIVE_TTL));
        }
        let bytes = response
            .error_for_status()
            .map_err(error)?
            .bytes()
            .await
            .map_err(error)?;
        let duration = f64::from(q.duration_s);
        tokio::task::spawn_blocking(move || {
            let results: Vec<Response> = serde_json::from_slice(&bytes).map_err(error)?;
            Ok((
                results
                    .into_iter()
                    .filter(|r| {
                        (duration == 0.0 || (r.duration - duration).abs() <= 3.0)
                            && (r.has_synced() || r.has_plain() || r.instrumental)
                    })
                    .min_by(|a, b| {
                        b.has_synced().cmp(&a.has_synced()).then_with(|| {
                            (a.duration - duration)
                                .abs()
                                .total_cmp(&(b.duration - duration).abs())
                        })
                    }),
                NEGATIVE_TTL,
            ))
        })
        .await
        .map_err(error)?
    }
}
fn transient(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
#[async_trait]
impl LyricsService for LrcLibClient {
    async fn lyrics(&self, q: &LyricsQuery) -> Result<Option<Lyrics>> {
        if q.title.trim().is_empty() || q.artist.trim().is_empty() {
            return Ok(None);
        }
        let path = self.cache_path(q);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let entry =
                    tokio::task::spawn_blocking(move || serde_json::from_slice::<Entry>(&bytes))
                        .await
                        .map_err(error)?;
                match entry {
                    Ok(entry)
                        if entry.lyrics.is_some()
                            || now().saturating_sub(entry.saved_at) < entry.negative_ttl =>
                    {
                        return Ok(entry.lyrics);
                    }
                    _ => {}
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(%e, "could not read lyrics cache"),
        }
        let (response, negative_ttl) = self.fetch(q).await?;
        tokio::task::spawn_blocking(move || {
            let lyrics = response
                .map(Response::lyrics)
                .filter(|l| l.instrumental || !l.synced.is_empty() || l.plain.is_some());
            let bytes = serde_json::to_vec(&Entry {
                saved_at: now(),
                negative_ttl,
                lyrics: lyrics.clone(),
            })
            .map_err(error)?;
            if let Err(e) = atomic_write(&path, &bytes) {
                tracing::warn!(%e, "could not save lyrics cache");
            }
            Ok(lyrics)
        })
        .await
        .map_err(error)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path, query_param},
    };
    fn query() -> LyricsQuery {
        LyricsQuery {
            title: "Song & title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_s: 100,
        }
    }
    #[tokio::test]
    async fn empty_queries_do_not_use_network() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        for (title, artist) in [
            ("", "Artist"),
            ("Song", ""),
            ("  ", "Artist"),
            ("Song", "\t"),
        ] {
            let mut q = query();
            q.title = title.into();
            q.artist = artist.into();
            assert_eq!(client.lyrics(&q).await.unwrap(), None);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn zero_duration_searches_without_duration_and_keeps_synced_preference() {
        let server = MockServer::start().await;
        Mock::given(path("/api/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"duration":0,"plainLyrics":"plain"}, {"duration":123,"syncedLyrics":"[00:01]synced"}
            ]))).expect(1).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        let mut q = query();
        q.duration_s = 0;
        assert_eq!(
            client.lyrics(&q).await.unwrap().unwrap().synced[0].text,
            "synced"
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0]
                .url
                .query_pairs()
                .any(|(key, _)| key == "duration")
        );
        server.verify().await;
    }
    #[tokio::test]
    async fn transient_errors_on_get_and_search_have_short_expiring_negatives() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        for endpoint in ["get", "search"] {
            for status in [429, 500, 503, 599] {
                server.reset().await;
                if endpoint == "search" {
                    Mock::given(path("/api/get"))
                        .respond_with(ResponseTemplate::new(400))
                        .expect(2)
                        .mount(&server)
                        .await;
                }
                Mock::given(path(format!("/api/{endpoint}")))
                    .respond_with(ResponseTemplate::new(status))
                    .expect(2)
                    .mount(&server)
                    .await;
                let mut q = query();
                q.title = format!("{endpoint}-{status}");
                let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
                assert!(client.lyrics(&q).await.unwrap().is_none());
                let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
                assert!(client.lyrics(&q).await.unwrap().is_none());
                let path = client.cache_path(&q);
                let mut entry: Entry =
                    serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
                assert_eq!(entry.negative_ttl, TRANSIENT_TTL);
                entry.saved_at = now() - TRANSIENT_TTL - 1;
                tokio::fs::write(&path, serde_json::to_vec(&entry).unwrap())
                    .await
                    .unwrap();
                assert!(client.lyrics(&q).await.unwrap().is_none());
                server.verify().await;
            }
        }
    }
    #[tokio::test]
    async fn cache_write_failure_still_returns_fetched_lyrics() {
        let server = MockServer::start().await;
        Mock::given(path("/api/get"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"plainLyrics":"obtained"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("lyrics"), "blocks directory creation")
            .await
            .unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        assert_eq!(
            client
                .lyrics(&query())
                .await
                .unwrap()
                .unwrap()
                .plain
                .as_deref(),
            Some("obtained")
        );
        server.verify().await;
    }
    #[test]
    fn lrc_multiple_timestamps_offsets_fractions_metadata_and_pauses() {
        let lines = parse_lrc(
            "[ar:Artist]\n[ti:Title]\n[00:12.34][01:02.000]words\n[00:01.123]early\n[00:02]\n[offset:+100]\n[al:Album]",
        );
        assert_eq!(
            lines,
            vec![
                LyricLine {
                    time_ms: 1223,
                    text: "early".into()
                },
                LyricLine {
                    time_ms: 2100,
                    text: "".into()
                },
                LyricLine {
                    time_ms: 12440,
                    text: "words".into()
                },
                LyricLine {
                    time_ms: 62100,
                    text: "words".into()
                }
            ]
        );
        assert_eq!(
            parse_lrc("[00:00.020]zero\n[00:01.12]one\n[offset:-100]")[0].time_ms,
            0
        );
        assert_eq!(parse_lrc("[00:01.12]one\n[offset:-100]")[0].time_ms, 1020);
        assert!(parse_lrc("[ar:x]\n[99:99]bad\n[00:01.x]bad").is_empty());
        assert_eq!(parse_lrc("[00:01][Chorus]")[0].text, "[Chorus]");
    }
    #[tokio::test]
    async fn fallback_prefers_synced_then_closest_duration_and_caches() {
        let server = MockServer::start().await;
        Mock::given(path("/api/get"))
            .and(query_param("album_name", "Album"))
            .and(query_param("duration", "100"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/search")).and(query_param("track_name","Song & title")).and(query_param("artist_name","Artist")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"duration":100,"plainLyrics":"plain"}, {"duration":101,"syncedLyrics":"[00:02.00]chosen"}, {"duration":102,"syncedLyrics":"[00:02]further"}, {"duration":104,"syncedLyrics":"[00:02]outside"}
        ]))).expect(1).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        let lyrics = client.lyrics(&query()).await.unwrap().unwrap();
        assert_eq!(lyrics.synced[0].text, "chosen");
        tokio::fs::write(
            client.cache_path(&query()),
            serde_json::to_vec(&Entry {
                saved_at: 0,
                negative_ttl: NEGATIVE_TTL,
                lyrics: Some(lyrics.clone()),
            })
            .unwrap(),
        )
        .await
        .unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        assert_eq!(Some(lyrics), client.lyrics(&query()).await.unwrap());
        server.verify().await;
    }
    #[tokio::test]
    async fn negative_cache_and_expiration() {
        let server = MockServer::start().await;
        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(404))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/api/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(2)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        assert!(client.lyrics(&query()).await.unwrap().is_none());
        assert!(client.lyrics(&query()).await.unwrap().is_none());
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        assert!(client.lyrics(&query()).await.unwrap().is_none());
        tokio::fs::write(
            client.cache_path(&query()),
            serde_json::to_vec(&Entry {
                saved_at: now() - NEGATIVE_TTL - 1,
                negative_ttl: NEGATIVE_TTL,
                lyrics: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();
        assert!(client.lyrics(&query()).await.unwrap().is_none());
        server.verify().await;
    }
    #[tokio::test]
    async fn plain_fallback_and_instrumental() {
        let server = MockServer::start().await;
        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/search")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "duration":98,"plainLyrics":"words" },{ "duration":100,"plainLyrics":"nearest" }]))).expect(1).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let client = LrcLibClient::with_base_url(dir.path(), server.uri()).unwrap();
        assert_eq!(
            client
                .lyrics(&query())
                .await
                .unwrap()
                .unwrap()
                .plain
                .as_deref(),
            Some("nearest")
        );
        server.verify().await;
        server.reset().await;
        Mock::given(path("/api/get"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"instrumental":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut q = query();
        q.title = "Instrumental".into();
        assert!(client.lyrics(&q).await.unwrap().unwrap().instrumental);
        server.verify().await;
    }
}
