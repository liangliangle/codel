use std::sync::Arc;

use base64::Engine as _;
use chrono::{Duration, Utc};
use codel_test_support::EnvGuard;
use codel_tools::implementations::codel_build::image_gen::{ImageGenClient, ImageGenConfig};
use codel_tools::implementations::codel_build::media_bearer::SIDE_CALL_BEARER_ERROR_CODE;
use codel_tools::types::api_key_provider::SideCallBearerError;
use pretty_assertions::assert_eq;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{SharedAuthKeyProvider, is_codel_side_call_principal};
use crate::config::CodelComConfig;
use crate::{AuthManager, AuthMode, CodelAuth};

const FOREIGN_ISSUER: &str = "https://cursor.com";

fn unsigned_jwt(header: &str, payload: &str) -> String {
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("{}.{}.sig", enc.encode(header), enc.encode(payload))
}
