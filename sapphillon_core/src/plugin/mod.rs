//! Plugin traits and concrete types.
//!
//! # Key difference from the original spec
//! `get_opdecl() -> OpDecl` is gone — `deno_core` types must not cross the
//! ABI boundary.  Instead, each function exposes a `call()` method that
//! `sapphillon_core::runtime` wraps into the `PluginDispatcher` callback.
//! The JS global `__sapphillon_dispatch` (in `sapphillon_js`) routes
//! every JS call to the right handler through that callback.

use std::sync::Arc;
use serde_json::Value;

// ── Traits ────────────────────────────────────────────────────────────────

pub trait PluginPackageTrait: Send + Sync {
    fn is_external(&self) -> bool;
    fn get_package_id(&self) -> String;
    fn get_package_name(&self) -> String;
    fn get_functions(&self) -> Vec<Box<dyn PluginFunctionTrait>>;
    fn as_external_package(&self) -> Option<&CorePluginExternalPackage>;
}

pub trait PluginFunctionTrait: Send + Sync {
    fn is_external(&self) -> bool;
    fn get_function_id(&self) -> String;
    fn get_function_name(&self) -> String;

    /// Handle a call from JS.
    ///
    /// `args` — the JSON value sent from the JS shim.
    /// Returns the JSON value to send back, or an error string.
    ///
    /// For *external* functions this is handled by `sapphillon_core::runtime`
    /// (which calls Extism) — the trait impl here is unused and may panic.
    fn call(&self, args: Value) -> Result<Value, String>;

    /// Optional JavaScript to run before the main workflow script.
    /// Used to install the `Sapphillon.<pkg>.<func>` shim that calls
    /// `__sapphillon_dispatch`.
    fn get_pre_run_js(&self) -> Option<String>;
}

// ── PluginIdentifier ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginIdentifier {
    pub package_id: String,
    pub function_name: String,
}

// ── Internal plugin ───────────────────────────────────────────────────────

/// A plugin function implemented as a native Rust closure/function.
///
/// The `func` field is the actual handler; everything else is metadata.
pub struct CorePluginFunction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub func: Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>,
    pub pre_run_js: Option<String>,
}

impl CorePluginFunction {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        func: impl Fn(Value) -> Result<Value, String> + Send + Sync + 'static,
        pre_run_js: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            func: Arc::new(func),
            pre_run_js,
        }
    }
}

impl PluginFunctionTrait for CorePluginFunction {
    fn is_external(&self) -> bool { false }
    fn get_function_id(&self) -> String { self.id.clone() }
    fn get_function_name(&self) -> String { self.name.clone() }
    fn call(&self, args: Value) -> Result<Value, String> { (self.func)(args) }
    fn get_pre_run_js(&self) -> Option<String> { self.pre_run_js.clone() }
}

/// A package whose functions are native Rust handlers.
pub struct CorePluginPackage {
    pub id: String,
    pub name: String,
    pub functions: Vec<CorePluginFunction>,
}

impl CorePluginPackage {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        functions: Vec<CorePluginFunction>,
    ) -> Self {
        Self { id: id.into(), name: name.into(), functions }
    }
}

impl PluginPackageTrait for CorePluginPackage {
    fn is_external(&self) -> bool { false }
    fn get_package_id(&self) -> String { self.id.clone() }
    fn get_package_name(&self) -> String { self.name.clone() }
    fn get_functions(&self) -> Vec<Box<dyn PluginFunctionTrait>> {
        self.functions
            .iter()
            .map(|f| -> Box<dyn PluginFunctionTrait> {
                Box::new(InternalFnRef {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    func: Arc::clone(&f.func),
                    pre_run_js: f.pre_run_js.clone(),
                })
            })
            .collect()
    }
    fn as_external_package(&self) -> Option<&CorePluginExternalPackage> { None }
}

struct InternalFnRef {
    id: String,
    name: String,
    func: Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>,
    pre_run_js: Option<String>,
}

impl PluginFunctionTrait for InternalFnRef {
    fn is_external(&self) -> bool { false }
    fn get_function_id(&self) -> String { self.id.clone() }
    fn get_function_name(&self) -> String { self.name.clone() }
    fn call(&self, args: Value) -> Result<Value, String> { (self.func)(args) }
    fn get_pre_run_js(&self) -> Option<String> { self.pre_run_js.clone() }
}

// ── External plugin (Extism / WASM) ──────────────────────────────────────

/// A single function in a WASM-backed external package.
///
/// Permissions are **not** checked here — the Extism WASM sandbox enforces
/// them when `extplugin_client` builds the `Manifest` with WASI capabilities.
pub struct CorePluginExternalFunction {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author_id: String,
    /// Parent package ID — used as the routing key: `"<package_id>::<name>"`.
    pub package_id: String,
}

impl PluginFunctionTrait for CorePluginExternalFunction {
    fn is_external(&self) -> bool { true }
    fn get_function_id(&self) -> String { self.id.clone() }
    fn get_function_name(&self) -> String { self.name.clone() }

    fn call(&self, _args: Value) -> Result<Value, String> {
        // Routing for external functions happens inside the dispatcher in
        // `sapphillon_core::runtime`, not here.
        unreachable!("external function call() should never be invoked directly")
    }

    /// JS shim that routes through `__sapphillon_dispatch`.
    ///
    /// Op key format: `"<package_id>::<function_name>"`.
    fn get_pre_run_js(&self) -> Option<String> {
        let pkg = &self.package_id;
        let func = &self.name;
        let op_key = format!("{pkg}::{func}");
        Some(format!(
            r#"
globalThis.Sapphillon = globalThis.Sapphillon ?? {{}};
globalThis.Sapphillon["{pkg}"] = globalThis.Sapphillon["{pkg}"] ?? {{}};
globalThis.Sapphillon["{pkg}"]["{func}"] = function(args) {{
    const raw = __sapphillon_dispatch(
        "{op_key}",
        JSON.stringify(args ?? {{}})
    );
    return JSON.parse(raw);
}};
"#
        ))
    }
}

/// A WASM-backed external plugin package.
pub struct CorePluginExternalPackage {
    pub id: String,
    pub name: String,
    pub functions: Vec<CorePluginExternalFunction>,
    /// Filesystem path to the `.wasm` file.
    /// Replaces the original `package_js` (JS source in a subprocess).
    pub wasm_path: String,
}

impl CorePluginExternalPackage {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        functions: Vec<CorePluginExternalFunction>,
        wasm_path: impl Into<String>,
    ) -> Self {
        Self { id: id.into(), name: name.into(), functions, wasm_path: wasm_path.into() }
    }
}

impl PluginPackageTrait for CorePluginExternalPackage {
    fn is_external(&self) -> bool { true }
    fn get_package_id(&self) -> String { self.id.clone() }
    fn get_package_name(&self) -> String { self.name.clone() }
    fn get_functions(&self) -> Vec<Box<dyn PluginFunctionTrait>> {
        self.functions
            .iter()
            .map(|f| -> Box<dyn PluginFunctionTrait> {
                Box::new(ExternalFnRef {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    package_id: f.package_id.clone(),
                })
            })
            .collect()
    }
    fn as_external_package(&self) -> Option<&CorePluginExternalPackage> { Some(self) }
}

struct ExternalFnRef {
    id: String,
    name: String,
    package_id: String,
}

impl PluginFunctionTrait for ExternalFnRef {
    fn is_external(&self) -> bool { true }
    fn get_function_id(&self) -> String { self.id.clone() }
    fn get_function_name(&self) -> String { self.name.clone() }
    fn call(&self, _args: Value) -> Result<Value, String> {
        unreachable!("external function call() should never be invoked directly")
    }
    fn get_pre_run_js(&self) -> Option<String> {
        let pkg = &self.package_id;
        let func = &self.name;
        let op_key = format!("{pkg}::{func}");
        Some(format!(
            r#"
globalThis.Sapphillon = globalThis.Sapphillon ?? {{}};
globalThis.Sapphillon["{pkg}"] = globalThis.Sapphillon["{pkg}"] ?? {{}};
globalThis.Sapphillon["{pkg}"]["{func}"] = function(args) {{
    const raw = __sapphillon_dispatch(
        "{op_key}",
        JSON.stringify(args ?? {{}})
    );
    return JSON.parse(raw);
}};
"#
        ))
    }
}
