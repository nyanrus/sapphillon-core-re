//! JS runtime orchestration.
//!
//! # Responsibilities
//! * Load the `sapphillon_deno` cdylib once per process.
//! * Build a [`PluginDispatcher`] callback that routes every `op_sapphillon_dispatch`
//!   call to the correct handler:
//!   - Internal functions → direct `PluginFunctionTrait::call()`.
//!   - External functions → `ext_plugin::extplugin_client` (Extism WASM sandbox).
//! * Thread `OpStateWorkflowData` through the call so consumers can retrieve
//!   captured stdout and metadata after execution.
//!
//! # Permission enforcement
//! **Permissions are enforced by Extism, not by this module.**  When the
//! dispatcher routes an external call, it passes the granted permissions to
//! `extplugin_client`, which builds an Extism `Manifest` with the corresponding
//! WASI capability grants.  The WASM sandbox then enforces those limits at the
//! syscall level — any filesystem or network access outside the granted set is
//! blocked by the sandbox, not by a Rust pre-check.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use abi_stable::std_types::{RErr, ROk, RResult, RStr, RString};
use ext_plugin::{extplugin_client, RsJsBridgeArgs, SapphillonPackage};
use sapphillon_deno_interface::{DropCtxFn, JsEngineLibRef, PluginDispatcher};

use crate::{
    error::{Error, SapphillonError, WorkflowRuntimeError, WorkflowRuntimeErrorType},
    permission::{find_allowed_permissions, PluginFunctionPermissions},
    plugin::{CorePluginExternalPackage, PluginFunctionTrait},
};

// ── JS engine dylib (loaded once) ─────────────────────────────────────────

static JS_ENGINE: OnceLock<JsEngineLibRef> = OnceLock::new();

/// Load the `sapphillon_deno` cdylib from `dylib_path`.
///
/// Must be called once before any workflow is run.  Subsequent calls with
/// different paths are ignored (the engine is already loaded).
pub fn init_js_engine(dylib_path: &str) -> Result<(), abi_stable::library::LibraryError> {
    if JS_ENGINE.get().is_some() {
        return Ok(());
    }
    let lib = JsEngineLibRef::load_from_file(dylib_path)?;
    JS_ENGINE.set(lib).ok();
    Ok(())
}

fn js_engine() -> &'static JsEngineLibRef {
    JS_ENGINE.get().expect(
        "JS engine not initialised — call sapphillon_core::runtime::init_js_engine(path) first",
    )
}

// ── WorkflowStdout ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WorkflowStdout {
    Stdout(String),
}

// ── OpStateWorkflowData ───────────────────────────────────────────────────

/// Shared workflow context.  Consumers can inspect captured stdout and
/// metadata after `run_script` returns.
pub struct OpStateWorkflowData {
    workflow_id: String,
    pub capture_stdout: bool,
    pub stdout_buf: Vec<WorkflowStdout>,
    pub allowed_permissions: Option<Vec<PluginFunctionPermissions>>,
    pub required_permissions: Option<Vec<PluginFunctionPermissions>>,
    pub external_packages: Vec<Arc<CorePluginExternalPackage>>,
}

impl OpStateWorkflowData {
    pub fn new(
        workflow_id: &str,
        capture_stdout: bool,
        allowed_permissions: Option<Vec<PluginFunctionPermissions>>,
        required_permissions: Option<Vec<PluginFunctionPermissions>>,
        external_packages: Vec<Arc<CorePluginExternalPackage>>,
    ) -> Self {
        Self {
            workflow_id: workflow_id.to_string(),
            capture_stdout,
            stdout_buf: Vec::new(),
            allowed_permissions,
            required_permissions,
            external_packages,
        }
    }

    pub fn get_workflow_id(&self) -> &str { &self.workflow_id }
    pub fn add_result(&mut self, s: WorkflowStdout) { self.stdout_buf.push(s); }
    pub fn get_results(&self) -> &Vec<WorkflowStdout> { &self.stdout_buf }
    pub fn is_capture_stdout(&self) -> bool { self.capture_stdout }

    pub fn stdout_to_string(&self) -> String {
        self.stdout_buf
            .iter()
            .map(|s| match s { WorkflowStdout::Stdout(t) => t.as_str() })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── Dispatcher context ────────────────────────────────────────────────────

/// The concrete type behind the `*const ()` context pointer in
/// [`PluginDispatcher`].  Lives on the heap for the duration of one
/// `run_script` call and is freed by the `drop_fn` callback.
struct DispatchCtx {
    /// Internal plugins indexed by `function_id`.
    internal: HashMap<String, Box<dyn PluginFunctionTrait>>,
    /// External (Extism) packages indexed by `package_id`.
    external: Vec<Arc<CorePluginExternalPackage>>,
    /// Permissions granted per plugin function ID.
    allowed_permissions: Vec<PluginFunctionPermissions>,
    /// Accumulates lines emitted by `op_capture_stdout`.
    stdout: Vec<String>,
}

extern "C" fn dispatch_call(
    ctx: *const (),
    op_key: RStr<'_>,
    args_json: RStr<'_>,
) -> RResult<RString, RString> {
    // SAFETY: `ctx` was created by `Box::into_raw` below and is still live.
    let ctx = unsafe { &*(ctx as *const DispatchCtx) };
    let op_key = op_key.as_str();
    let args_str = args_json.as_str();

    match dispatch_impl(ctx, op_key, args_str) {
        Ok(s)  => ROk(RString::from(s)),
        Err(e) => RErr(RString::from(e)),
    }
}

extern "C" fn drop_ctx(ctx: *const ()) {
    // SAFETY: same box created below.
    unsafe { drop(Box::from_raw(ctx as *mut DispatchCtx)) };
}

fn dispatch_impl(ctx: &DispatchCtx, op_key: &str, args_json: &str) -> Result<String, String> {
    // ── Internal plugin ───────────────────────────────────────────────
    if let Some(func) = ctx.internal.get(op_key) {
        let args: serde_json::Value =
            serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
        let result = func.call(args).map_err(|e| e)?;
        return serde_json::to_string(&result).map_err(|e| e.to_string());
    }

    // ── External plugin (Extism) ──────────────────────────────────────
    // Op key for external functions: "<package_id>::<function_name>"
    if let Some((pkg_id, func_name)) = op_key.split_once("::") {
        if let Some(pkg) = ctx.external.iter().find(|p| p.id == pkg_id) {
            // Gather the permissions Extism may grant for this function.
            // Extism enforces them via WASI capability grants — no Rust check.
            let allowed = find_allowed_permissions(
                &ctx.allowed_permissions,
                &[func_name],
            );

            let args_map: indexmap::IndexMap<String, serde_json::Value> =
                serde_json::from_str(args_json).unwrap_or_default();
            let bridge_args = RsJsBridgeArgs {
                func_name: func_name.to_string(),
                args: args_map,
            };

            let sapphillon_pkg = SapphillonPackage::from_wasm_path(&pkg.wasm_path)
                .map_err(|e| format!("failed to load wasm '{}': {e}", pkg.wasm_path))?;

            let returns =
                extplugin_client(&sapphillon_pkg, func_name, &bridge_args, allowed.permissions)
                    .map_err(|e| format!("extism call failed: {e}"))?;

            return serde_json::to_string(&returns.args).map_err(|e| e.to_string());
        }
    }

    Err(format!("unknown op key: {op_key}"))
}

// ── run_script ────────────────────────────────────────────────────────────

/// Execute a JavaScript workflow script through the Deno cdylib.
///
/// # Steps
/// 1. Build a [`PluginDispatcher`] routing table from `plugins`.
/// 2. Collect `get_pre_run_js()` shims from every plugin function.
/// 3. Call the Deno dylib's `run_script`; it runs shims then `script`.
/// 4. Ingest captured stdout into `workflow_data`, return it.
///
/// # Note on permissions
/// There is no pre-flight permission check here.  The Extism WASM sandbox
/// enforces permissions at the syscall boundary when each external function is
/// called.  See `ext_plugin::extplugin_client` for details.
pub fn run_script(
    script: &str,
    plugins: Vec<Box<dyn PluginFunctionTrait>>,
    workflow_data: Option<Arc<Mutex<OpStateWorkflowData>>>,
    pre_script: Option<Vec<String>>,
) -> Result<Arc<Mutex<OpStateWorkflowData>>, Box<SapphillonError>> {
    let engine = js_engine();

    // ── Build the default workflow_data if not provided ───────────────
    let data_arc = workflow_data.unwrap_or_else(|| {
        Arc::new(Mutex::new(OpStateWorkflowData::new(
            "__default__",
            true,
            None,
            None,
            vec![],
        )))
    });

    // ── Collect pre-scripts (plugin shims) ────────────────────────────
    let mut pre_scripts: Vec<abi_stable::std_types::RString> = pre_script
        .unwrap_or_default()
        .into_iter()
        .map(abi_stable::std_types::RString::from)
        .collect();

    // ── Build routing table + gather more shims from plugins ──────────
    let mut internal: HashMap<String, Box<dyn PluginFunctionTrait>> = HashMap::new();

    for func in plugins {
        if let Some(js) = func.get_pre_run_js() {
            pre_scripts.push(abi_stable::std_types::RString::from(js));
        }
        if !func.is_external() {
            internal.insert(func.get_function_id(), func);
        }
        // External functions are routed by the package lookup in dispatch_impl.
    }

    // ── Build DispatchCtx ─────────────────────────────────────────────
    let (external_packages, allowed_permissions) = {
        let data = data_arc.lock().unwrap();
        (
            data.external_packages.clone(),
            data.allowed_permissions.clone().unwrap_or_default(),
        )
    };

    let ctx = Box::new(DispatchCtx {
        internal,
        external: external_packages,
        allowed_permissions,
        stdout: Vec::new(),
    });
    let ctx_ptr = Box::into_raw(ctx) as *const ();

    let dispatcher = PluginDispatcher {
        ctx: ctx_ptr,
        call_fn: dispatch_call,
        drop_fn: drop_ctx as DropCtxFn,
    };

    // ── Call into the Deno dylib ──────────────────────────────────────
    let result =
        (engine.run_script())(RStr::from_str(script), pre_scripts.into(), dispatcher);

    // ── Ingest result into workflow_data ──────────────────────────────
    match result {
        ROk(stdout) => {
            let mut data = data_arc.lock().unwrap();
            for line in stdout.as_str().split('\n') {
                if !line.is_empty() {
                    data.add_result(WorkflowStdout::Stdout(line.to_string()));
                }
            }
            Ok(data_arc.clone())
        }
        RErr(err_msg) => {
            let msg = err_msg.into_string();
            Err(Box::new(Error::WorkflowRuntimeError(WorkflowRuntimeError {
                message: msg.lines().next().unwrap_or("").to_string(),
                error_type: WorkflowRuntimeErrorType::WorkflowScriptExecuteError,
                js_error_detail: msg,
            })))
        }
    }
}
