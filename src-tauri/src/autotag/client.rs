//! HTTP client for the analyzer service.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::plan::{ScoreRequest, ScoreResponse};

/// Header that carries the shared secret the analyzer checks.
const TOKEN_HEADER: &str = "X-LuteDrop-Token";

/// How long to wait for the TCP connection before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// `/health` answers immediately, so it gets a short overall limit.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
/// `/embed` decodes and embeds up to 32 files, which can take minutes.
const EMBED_TIMEOUT: Duration = Duration::from_secs(300);
/// `/score` trains and scores every classifier from cached embeddings.
const SCORE_TIMEOUT: Duration = Duration::from_secs(120);

/// The analyzer's verdict on one item of an `/embed` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedResult {
    pub hash: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// The three calls LuteDrop makes to the analyzer. Tests swap in a fake.
pub trait AnalyzerClient: Send + Sync {
    /// Checks the analyzer is up. Returns a short description of it.
    fn health(&self) -> Result<String, String>;
    /// Embeds up to 32 files, given as (content hash, path on the analyzer host).
    fn embed(&self, items: &[(String, String)]) -> Result<Vec<EmbedResult>, String>;
    /// Trains on the examples in `req` and scores its targets.
    fn score(&self, req: &ScoreRequest) -> Result<ScoreResponse, String>;
}

/// `AnalyzerClient` over HTTP, using blocking `ureq` calls.
pub struct HttpAnalyzer {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl HttpAnalyzer {
    /// `base` is the analyzer URL, such as `http://gargantua:8765`.
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            base: base.trim().trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent: ureq::AgentBuilder::new().timeout_connect(CONNECT_TIMEOUT).build(),
        }
    }

    fn url(&self, path: &str) -> Result<String, String> {
        if self.base.is_empty() {
            return Err("No analyzer URL is set".to_string());
        }
        Ok(format!("{}{}", self.base, path))
    }
}

#[derive(Deserialize)]
struct HealthResponse {
    model: String,
    cache_count: u64,
}

#[derive(Serialize)]
struct EmbedItem<'a> {
    hash: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    items: Vec<EmbedItem<'a>>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    results: Vec<EmbedResult>,
}

/// Turns a `ureq` error into a message for the user. For an HTTP error
/// status, the analyzer's response body is included because it says why.
fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            format!("Analyzer returned HTTP {code}: {}", body.trim())
        }
        ureq::Error::Transport(t) => format!("Could not reach the analyzer: {t}"),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(response: ureq::Response) -> Result<T, String> {
    response
        .into_json()
        .map_err(|e| format!("Analyzer sent an unreadable response: {e}"))
}

impl AnalyzerClient for HttpAnalyzer {
    fn health(&self) -> Result<String, String> {
        let response = self
            .agent
            .get(&self.url("/health")?)
            .set(TOKEN_HEADER, &self.token)
            .timeout(HEALTH_TIMEOUT)
            .call()
            .map_err(describe)?;
        let health: HealthResponse = read_json(response)?;
        Ok(format!(
            "Connected: model {}, {} cached embeddings",
            health.model, health.cache_count
        ))
    }

    fn embed(&self, items: &[(String, String)]) -> Result<Vec<EmbedResult>, String> {
        let body = EmbedRequest {
            items: items
                .iter()
                .map(|(hash, path)| EmbedItem { hash, path })
                .collect(),
        };
        let response = self
            .agent
            .post(&self.url("/embed")?)
            .set(TOKEN_HEADER, &self.token)
            .timeout(EMBED_TIMEOUT)
            .send_json(&body)
            .map_err(describe)?;
        let parsed: EmbedResponse = read_json(response)?;
        Ok(parsed.results)
    }

    fn score(&self, req: &ScoreRequest) -> Result<ScoreResponse, String> {
        let response = self
            .agent
            .post(&self.url("/score")?)
            .set(TOKEN_HEADER, &self.token)
            .timeout(SCORE_TIMEOUT)
            .send_json(req)
            .map_err(describe)?;
        read_json(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_is_an_error_not_a_request() {
        let client = HttpAnalyzer::new("  ", "t");
        assert_eq!(client.health(), Err("No analyzer URL is set".to_string()));
    }

    #[test]
    fn trailing_slash_is_dropped_from_base() {
        let client = HttpAnalyzer::new("http://host:8765/", "t");
        assert_eq!(client.url("/score").unwrap(), "http://host:8765/score");
    }

    #[test]
    fn embed_response_parses_with_and_without_error() {
        let json = r#"{"results":[{"hash":"a","ok":true},{"hash":"b","ok":false,"error":"decode failed"}]}"#;
        let parsed: EmbedResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.results[0].error, None);
        assert_eq!(parsed.results[1].error.as_deref(), Some("decode failed"));
    }
}
