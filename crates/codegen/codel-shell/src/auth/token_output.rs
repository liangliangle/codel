use chrono::{DateTime, Duration, Utc};

/// Parsed output from an external auth provider command.
#[derive(Debug, Clone)]
pub struct TokenOutput {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<String>,
}

/// Compute an expiry timestamp from a TTL in seconds.
pub fn expiry_after_seconds(secs: u64) -> Option<DateTime<Utc>> {
    Some(Utc::now() + Duration::seconds(secs as i64))
}

/// Parse stdout from an external auth provider command into a [`TokenOutput`].
pub fn parse_token_output(output: &std::process::Output) -> anyhow::Result<TokenOutput> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let token = stdout.trim();
    if token.is_empty() {
        anyhow::bail!("auth provider produced empty output");
    }
    Ok(TokenOutput {
        access_token: token.to_owned(),
        expires_at: None,
        refresh_token: None,
    })
}
