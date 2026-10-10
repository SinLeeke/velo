//! Spotify's web player GraphQL (`api-partner.spotify.com/pathfinder`), with the session's
//! login5 token and client token. Persisted query hashes change when Spotify updates its web
//! player: callers must treat any error as "fall back".

use std::time::Duration;

use bytes::Bytes;
use http::{HeaderValue, Method, Request, header::ACCEPT, header::AUTHORIZATION};
use librespot_core::Session;
use serde_json::{Value, json};
use velo_domain::{Result, VeloError};

use super::unavailable;

const URL: &str = "https://api-partner.spotify.com/pathfinder/v1/query";
const TIMEOUT: Duration = Duration::from_secs(15);

/// The query URL (no token in it).
pub fn query_url(operation: &str, variables: &Value, sha256: &str) -> String {
    let extensions = json!({"persistedQuery": {"version": 1, "sha256Hash": sha256}});
    let mut url = url_with(URL);
    url.query_pairs_mut()
        .append_pair("operationName", operation)
        .append_pair("variables", &variables.to_string())
        .append_pair("extensions", &extensions.to_string());
    url.to_string()
}

fn url_with(base: &str) -> url::Url {
    url::Url::parse(base).expect("static pathfinder URL")
}

/// `data` of a persisted query; errors (including GraphQL `errors` without data) are
/// `VeloError::Network`, never the response text.
pub async fn query(
    session: &Session,
    operation: &str,
    variables: &Value,
    sha256: &str,
) -> Result<Value> {
    let url = query_url(operation, variables, sha256);
    tokio::time::timeout(TIMEOUT, async {
        let token = session
            .login5()
            .auth_token()
            .await
            .map_err(|_| unavailable())?;
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(url)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(
                AUTHORIZATION,
                format!("{} {}", token.token_type, token.access_token),
            );
        if let Ok(client_token) = session.spclient().client_token().await {
            request = request.header("client-token", client_token);
        }
        let request = request.body(Bytes::new()).map_err(|_| unavailable())?;
        let body = session
            .http_client()
            .request_body(request)
            .await
            .map_err(|_| unavailable())?;
        data_of(&body)
    })
    .await
    .map_err(|_| VeloError::Network("pathfinder timed out".into()))?
}

/// The `data` object of a GraphQL answer.
pub fn data_of(body: &[u8]) -> Result<Value> {
    let mut value: Value = serde_json::from_slice(body).map_err(|_| unavailable())?;
    match value.get_mut("data").map(Value::take) {
        Some(data) if !data.is_null() => Ok(data),
        _ => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_carry_the_persisted_query_and_answers_need_data() {
        let url = query_url("profileAttributes", &json!({}), "abc");
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(pairs["operationName"], "profileAttributes");
        assert_eq!(pairs["variables"], "{}");
        assert!(pairs["extensions"].contains("\"sha256Hash\":\"abc\""));
        assert_eq!(
            data_of(br#"{"data":{"me":{"id":1}}}"#).unwrap()["me"]["id"],
            1
        );
        assert!(data_of(br#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#).is_err());
        assert!(data_of(br#"{"data":null}"#).is_err());
        assert!(data_of(b"<html>").is_err());
    }
}
