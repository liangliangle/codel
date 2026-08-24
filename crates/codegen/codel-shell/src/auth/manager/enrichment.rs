use std::sync::Arc;
use crate::auth::model::CodelAuth;

pub fn apply_user_info_enrichment(_auth: &mut CodelAuth) {}

pub fn spawn(_manager: Arc<super::AuthManager>, _auth: CodelAuth) {}

pub async fn enrich_inline(_manager: &super::AuthManager, _auth: &mut CodelAuth) {}
