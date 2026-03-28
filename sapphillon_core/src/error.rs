use thiserror::Error;
use crate::permission::Permissions;

pub type SapphillonError = Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("workflow runtime error: {0}")]
    WorkflowRuntimeError(#[from] WorkflowRuntimeError),

    #[error("permission denied: {0}")]
    PermissionDeniedError(#[from] PermissionDeniedError),
}

// ── Runtime error ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRuntimeErrorType {
    CorePluginPrepareError,
    CorePluginExecuteError,
    WorkflowScriptExecuteError,
}

/// A JavaScript execution failure.  The original JS error message and stack
/// trace are preserved as a plain string — `the JS engine` types never cross the
/// ABI boundary, so we capture them as text inside `sapphillon_js`.
#[derive(Debug, Error)]
#[error("{error_type:?}: {message}")]
pub struct WorkflowRuntimeError {
    pub message: String,
    pub error_type: WorkflowRuntimeErrorType,
    /// Full JS error message + stack trace formatted by `sapphillon_js`.
    pub js_error_detail: String,
}

// ── Permission denied error ───────────────────────────────────────────────

#[derive(Debug, Error)]
#[error("permission denied — requested: {requested:?}, granted: {granted:?}")]
pub struct PermissionDeniedError {
    pub requested: Permissions,
    pub granted: Permissions,
}
