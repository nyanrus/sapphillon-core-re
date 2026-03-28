//! Sapphillon JS runtime — Extism host cdylib.
//!
//! Replaces the previous `deno_core`/V8 implementation.  JavaScript workflows
//! now run inside a QuickJS-based Extism WASM component (`js_engine.wasm`,
//! compiled from `sapphillon_javy_plugin/plugin.js` via `extism-js`).
//!
//! # Dispatch protocol
//! JS calls `globalThis.__sapphillon_dispatch(opKey, argsJson)`, which the
//! plugin routes to the Extism host function `"dispatch"`.  The host function
//! decodes the request, calls `PluginDispatcher::call`, and returns a JSON
//! envelope `{"ok":"…"}` or `{"err":"…"}`.
//!
//! # ABI boundary
//! The public surface (`ffi_run_script`) is defined in `sapphillon_js_interface`
//! and consumed by `sapphillon_core` — neither crate changes when this one does.

use abi_stable::{
    export_root_module,
    prefix_type::PrefixTypeTrait,
    std_types::{RErr, ROk, RResult, RStr, RString, RVec},
};
use extism::{Function, Manifest, Plugin, UserData, Val, ValType, Wasm};
use sapphillon_js_interface::{JsEngineLib, JsEngineLibRef, PluginDispatcher};

// ── Embedded WASM ─────────────────────────────────────────────────────────
// Built once from sapphillon_javy_plugin/plugin.js by running:
//   extism-js plugin.js -o js_engine.wasm

static PLUGIN_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/js_engine.wasm"));

// ── Root module export (abi_stable) ───────────────────────────────────────

#[export_root_module]
pub fn get_root_module() -> JsEngineLibRef {
    JsEngineLib { run_script: ffi_run_script }.leak_into_prefix()
}

// ── FFI entry point ───────────────────────────────────────────────────────

extern "C" fn ffi_run_script(
    script: RStr<'_>,
    pre_scripts: RVec<RString>,
    dispatcher: PluginDispatcher,
) -> RResult<RString, RString> {
    let pre: Vec<String> = pre_scripts.iter().map(|s| s.to_string()).collect();

    match run_script_impl(script.as_str(), &pre, dispatcher) {
        Ok(stdout) => ROk(RString::from(stdout)),
        Err(e)     => RErr(RString::from(e)),
    }
}

// ── Dispatch host function ────────────────────────────────────────────────

// Called by the WASM plugin whenever JS invokes __sapphillon_dispatch.
//
// Input  (string): JSON `{"op_key":"<pkg>::<fn>","args_json":"{…}"}`
// Output (string): JSON `{"ok":"<result_json>"}` or `{"err":"<message>"}`
extism::host_fn!(sapphillon_dispatch_host(
    user_data: UserData<PluginDispatcher>,
    input: String
) -> String {
    let data       = user_data.get()?;
    let dispatcher = *data.lock().unwrap();

    let req: serde_json::Value = serde_json::from_str(&input)?;
    let op_key   = req["op_key"].as_str().unwrap_or_default().to_owned();
    let args_json = req["args_json"].as_str().unwrap_or("{}").to_owned();

    let result = dispatcher.call(
        RStr::from_str(&op_key),
        RStr::from_str(&args_json),
    );

    let envelope = match result {
        ROk(s) => serde_json::json!({ "ok":  s.as_str() }),
        RErr(e) => serde_json::json!({ "err": e.as_str() }),
    };

    Ok(serde_json::to_string(&envelope)?)
});

// ── Core implementation ───────────────────────────────────────────────────

fn run_script_impl(
    script: &str,
    pre_scripts: &[String],
    dispatcher: PluginDispatcher,
) -> Result<String, String> {
    // Build the dispatch host function, passing the dispatcher as user data.
    let dispatch_fn = Function::new(
        "dispatch",
        [ValType::I64],
        [ValType::I64],
        Some(UserData::new(dispatcher)),
        sapphillon_dispatch_host,
    );

    // Load the embedded WASM plugin.
    let wasm     = Wasm::data(PLUGIN_WASM);
    let manifest = Manifest::new([wasm]);
    let mut plugin = Plugin::new(&manifest, [dispatch_fn], true)
        .map_err(|e| format!("failed to load JS engine plugin: {e}"))?;

    // Serialise the call arguments.
    let input = serde_json::json!({
        "script":      script,
        "pre_scripts": pre_scripts,
    });
    let input_str = serde_json::to_string(&input)
        .map_err(|e| format!("failed to serialise script input: {e}"))?;

    // Run the workflow inside the WASM plugin.
    let stdout = plugin
        .call::<&str, &str>("run_script", &input_str)
        .map_err(|e| e.to_string())?;

    // Drop the dispatcher context now that the plugin call is complete.
    (dispatcher.drop_fn)(dispatcher.ctx);

    Ok(stdout.to_owned())
}
