//! The HTTP the daemon and the shell share: the web's API (`remux login`,
//! the session). What to send and how to read an answer is the domain's;
//! this only carries it.

use std::time::Duration;

/// What came back: the status and the body, JSON or `Null`.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub status: u16,
    pub body: serde_json::Value,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("no http client: {e}"))
}

fn answered(sent: Result<reqwest::blocking::Response, reqwest::Error>) -> Result<Answer, String> {
    let answer = sent.map_err(|e| e.without_url().to_string())?;
    let status = answer.status().as_u16();
    let text = answer.text().unwrap_or_default();
    Ok(Answer {
        status,
        body: serde_json::from_str(&text).unwrap_or(serde_json::Value::Null),
    })
}

/// One JSON question to a URL, one JSON answer.
pub fn post_json(url: &str, body: &serde_json::Value) -> Result<Answer, String> {
    answered(client()?.post(url).json(body).send())
}

/// One GET with a bearer token, one JSON answer.
pub fn get_json(url: &str, bearer: &str) -> Result<Answer, String> {
    answered(client()?.get(url).bearer_auth(bearer).send())
}
