//! Keep secrets out of error messages, logs and the UI.

/// Remove every occurrence of `secret` from `text` (tokens can appear in URLs
/// that HTTP errors echo back).
pub fn redact(text: &str, secret: &str) -> String {
    if secret.len() < 8 { text.to_string() } else { text.replace(secret, "[redacted]") }
}

/// Turn an error into a message that is safe to show or log.
pub fn safe_error(err: &anyhow::Error, secrets: &[&str]) -> anyhow::Error {
    let mut msg = format!("{err:#}");
    for s in secrets {
        msg = redact(&msg, s);
    }
    // Anything that still looks like a JWT (three base64url parts) goes too.
    let cleaned: Vec<String> = msg
        .split_whitespace()
        .map(|w| if looks_like_jwt(w) { "[redacted]".to_string() } else { w.to_string() })
        .collect();
    anyhow::anyhow!(cleaned.join(" "))
}

fn looks_like_jwt(word: &str) -> bool {
    word.split(['=', '&', '?']).any(|part| {
        let segs: Vec<&str> = part.split('.').collect();
        segs.len() == 3
            && segs
                .iter()
                .all(|s| s.len() >= 8 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_tokens_from_urls() {
        let e = anyhow::anyhow!(
            "GET https://x/?access_token=eyJhbGciOi.eyJzdWIiOiI3NjU.SflKxwRJSMeKKF2QT4fw&steamid=1 failed"
        );
        let out = format!("{}", safe_error(&e, &[]));
        assert!(!out.contains("eyJ"), "{out}");
        let out = format!("{}", safe_error(&anyhow::anyhow!("token supersecretvalue here"), &["supersecretvalue"]));
        assert_eq!(out, "token [redacted] here");
    }
}
