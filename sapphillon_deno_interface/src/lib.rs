//! Stable ABI contract between `sapphillon_core` and `sapphillon_deno`.
//!
//! Neither side imports `deno_core` through this crate — all communication
//! crosses the boundary as UTF-8 JSON strings via C-compatible function
//! pointers.
//!
//! # Why abi_stable?
//! `deno_core` pulls in V8, making it a very heavy compile unit.  Isolating
//! it inside a `cdylib` means the rest of the workspace can be rebuilt without
//! touching V8.  `abi_stable` gives us type-checked, versioned dynamic
//! linking in pure Rust.

use abi_stable::{
    declare_root_module_statics,
    library::RootModule,
    package_version_strings,
    sabi_types::VersionStrings,
    std_types::{RResult, RStr, RString, RVec},
    StableAbi,
};

// ── PluginDispatcher ──────────────────────────────────────────────────────
//
// An FFI-safe callback that `sapphillon_core` constructs and passes into the
// Deno dylib so the JS runtime can route `op_sapphillon_dispatch` calls back
// to whichever plugin (internal Rust or external Extism) owns the op.
//
// Layout: a fat pointer — raw context + `extern "C"` call/drop pair.

/// Opaque context pointer.  The dylib treats it as completely opaque; only
/// `sapphillon_core` knows the concrete type behind it.
pub type CtxPtr = *const ();

/// `call_fn(ctx, op_name, args_json) -> Ok(result_json) | Err(message)`
pub type DispatchCallFn =
    extern "C" fn(CtxPtr, RStr<'_>, RStr<'_>) -> RResult<RString, RString>;

/// Called when the dylib is done with the context (end of `run_script`).
pub type DropCtxFn = extern "C" fn(CtxPtr);

/// FFI-safe, stateful op dispatcher.
///
/// Constructed by `sapphillon_core::runtime` and handed to the Deno dylib for
/// the duration of a single `run_script` call.
#[repr(C)]
#[derive(Clone, Copy, StableAbi)]
pub struct PluginDispatcher {
    pub ctx:      CtxPtr,
    pub call_fn:  DispatchCallFn,
    pub drop_fn:  DropCtxFn,
}

// SAFETY: ctx is owned by the creator (sapphillon_core) which ensures no
// concurrent mutation during the dylib call.
unsafe impl Send for PluginDispatcher {}
unsafe impl Sync for PluginDispatcher {}

impl PluginDispatcher {
    #[inline]
    pub fn call(&self, op_name: RStr<'_>, args_json: RStr<'_>) -> RResult<RString, RString> {
        (self.call_fn)(self.ctx, op_name, args_json)
    }
}

// ── JsEngineLib / JsEngineLibRef ─────────────────────────────────────────
//
// The module exported from `sapphillon_deno.dll` / `libsapphillon_deno.so`.
// `sapphillon_core` loads it at runtime via `JsEngineLibRef::load_from_file`.

/// Prefix struct exported by the `sapphillon_deno` cdylib.
///
/// Adding new `#[sabi(not_prefix_field)]` fields to the *end* is
/// backward-compatible; existing callers keep working.
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = JsEngineLibRef)))]
pub struct JsEngineLib {
    /// Execute a JS workflow.
    ///
    /// - `script`       — the main workflow source.
    /// - `pre_scripts`  — JS snippets run before `script` (e.g. plugin shims).
    /// - `dispatcher`   — routes every `Sapphillon.*` op call to the right handler.
    ///
    /// Returns `Ok(captured_stdout)` or `Err(error_message + JS stack trace)`.
    #[sabi(last_prefix_field)]
    pub run_script: extern "C" fn(
        script: RStr<'_>,
        pre_scripts: RVec<RString>,
        dispatcher: PluginDispatcher,
    ) -> RResult<RString, RString>,
}

impl RootModule for JsEngineLibRef {
    declare_root_module_statics! { JsEngineLibRef }
    const BASE_NAME: &'static str = "sapphillon_deno";
    const NAME: &'static str = "sapphillon_deno";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
