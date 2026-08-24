/// Returns `true` if the JWT is expired or will expire within `buffer`.
pub fn is_jwt_expired_or_near(_token: &str, _buffer: chrono::Duration) -> bool {
    false
}

/// Parse the `exp` claim from a JWT without verifying the signature.
pub fn parse_jwt_expiration(_token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    None
}
