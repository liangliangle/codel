use std::sync::Arc;

use chrono::{Duration, Utc};
use pretty_assertions::assert_eq;
use codel_login::{AuthManager, AuthMode, CodelAuth, CodelComConfig};
use codel_test_support::EnvGuard;
use codel_voice::VoiceAuthError;

use super::build_voice_auth;


