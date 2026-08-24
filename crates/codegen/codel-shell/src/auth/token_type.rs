use crate::auth::model::CodelAuth;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    ApiKey,
    None,
}

impl TokenType {
    pub fn from_auth(auth: Option<&CodelAuth>) -> Self {
        match auth {
            Some(a) if matches!(a.auth_mode, crate::auth::model::AuthMode::ApiKey) => Self::ApiKey,
            _ => Self::None,
        }
    }

    pub fn is_refreshable(&self) -> bool {
        false
    }
}
