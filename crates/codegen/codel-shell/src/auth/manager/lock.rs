use std::path::Path;

use crate::auth::storage::AuthFileLock;

pub async fn try_lock_auth_file_async(
    _path: &Path,
    _timeout: std::time::Duration,
) -> Option<AuthFileLock> {
    None
}

pub fn try_lock_auth_file_nonblocking(_path: &Path) -> Option<AuthFileLock> {
    None
}
