//! `remux login`, the part that is a decision: what the web's two answers
//! mean. The shell sends the requests and sleeps; this reads.

use serde_json::Value;

/// What `POST /api/device` said: what to show, what to poll with.
#[derive(Debug, Clone, PartialEq)]
pub struct Started {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in_secs: u64,
    pub interval_secs: u64,
}

pub fn started(status: u16, body: &Value) -> Result<Started, String> {
    if status != 200 {
        return Err(format!("the web said {status} to a login"));
    }
    let text = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("the web's answer has no {key}"))
    };
    Ok(Started {
        device_code: text("device_code")?,
        user_code: text("user_code")?,
        verification_url: text("verification_url")?,
        expires_in_secs: body
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(600),
        interval_secs: body.get("interval").and_then(Value::as_u64).unwrap_or(3),
    })
}

/// What `POST /api/token` said, each poll.
#[derive(Debug, Clone, PartialEq)]
pub enum Polled {
    Pending,
    Token(String),
    Denied(String),
}

pub fn polled(status: u16, body: &Value) -> Polled {
    match (status, body.get("token").and_then(Value::as_str)) {
        (200, Some(token)) => Polled::Token(token.to_string()),
        (428, _) => Polled::Pending,
        (410, _) => Polled::Denied("the code expired before it was typed".into()),
        (status, _) => Polled::Denied(
            body.get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("the web said {status}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_start_is_read_whole_and_a_short_answer_is_refused() {
        let said = started(
            200,
            &json!({"device_code": "d", "user_code": "ABCD2345", "verification_url": "http://w/device", "expires_in": 600, "interval": 3}),
        )
        .unwrap();
        assert_eq!(
            (said.user_code.as_str(), said.interval_secs),
            ("ABCD2345", 3)
        );
        assert!(started(500, &json!({})).is_err());
        assert!(started(200, &json!({"device_code": "d"})).is_err());
    }

    #[test]
    fn each_poll_is_pending_a_token_or_a_refusal() {
        assert_eq!(
            polled(428, &json!({"error": "authorization_pending"})),
            Polled::Pending
        );
        assert_eq!(
            polled(200, &json!({"token": "t"})),
            Polled::Token("t".into())
        );
        assert!(matches!(polled(410, &json!({})), Polled::Denied(_)));
        assert_eq!(
            polled(401, &json!({"error": "nope"})),
            Polled::Denied("nope".into())
        );
    }
}
