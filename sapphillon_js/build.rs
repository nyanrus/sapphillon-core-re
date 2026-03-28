//! Embed the pre-compiled Extism JS-PDK WASM into this crate.
//!
//! The WASM must be built once from `sapphillon_javy_plugin/plugin.js`:
//!
//!   npm install -g @extism/js-pdk
//!   cd sapphillon_javy_plugin
//!   extism-js plugin.js -o js_engine.wasm
//!
//! After that, `cargo build` picks it up automatically.

use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let wasm_src     = manifest_dir.join("../sapphillon_javy_plugin/js_engine.wasm");
    let out_dir      = PathBuf::from(env::var("OUT_DIR").unwrap());
    let wasm_dst     = out_dir.join("js_engine.wasm");

    if !wasm_src.exists() {
        panic!(
            "\n\
            ────────────────────────────────────────────────────────\n\
            sapphillon_js build error: js_engine.wasm not found.\n\
            \n\
            Build it once from the JS plugin source:\n\
            \n\
              npm install -g @extism/js-pdk\n\
              cd sapphillon_javy_plugin\n\
              extism-js plugin.js -o js_engine.wasm\n\
            ────────────────────────────────────────────────────────\n"
        );
    }

    fs::copy(&wasm_src, &wasm_dst).expect("failed to copy js_engine.wasm to OUT_DIR");

    // Re-run if the plugin source or compiled WASM changes.
    println!("cargo:rerun-if-changed={}", wasm_src.display());
    println!("cargo:rerun-if-changed=../sapphillon_javy_plugin/plugin.js");
}
