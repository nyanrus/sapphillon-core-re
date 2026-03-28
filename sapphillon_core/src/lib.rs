pub mod error;
pub mod permission;
pub mod plugin;
pub mod runtime;
pub mod utils;
pub mod workflow;

// ── Public re-exports ─────────────────────────────────────────────────────

pub use workflow::CoreWorkflowCode;

pub use plugin::{
    CorePluginExternalFunction, CorePluginExternalPackage,
    CorePluginFunction, CorePluginPackage,
    PluginFunctionTrait, PluginIdentifier, PluginPackageTrait,
};

pub use permission::{
    check_permission, find_allowed_permissions,
    CheckPermissionResult, PluginFunctionPermissions, Permissions,
};

pub use runtime::{init_js_engine, OpStateWorkflowData, WorkflowStdout, run_script};

pub use error::{
    Error, PermissionDeniedError, SapphillonError,
    WorkflowRuntimeError, WorkflowRuntimeErrorType,
};

pub use utils::{
    check_path::{paths_cover_as_set, paths_cover_by_ancestor},
    check_url::urls_cover_by_ancestor,
};
