//! External plugin subsystem — powered by Extism (WASM sandbox).
//!
//! # Key differences from the original design
//!
//! | Original                        | This implementation               |
//! |---------------------------------|-----------------------------------|
//! | JS source (`package_js`)        | WASM module path (`wasm_path`)    |
//! | Subprocess spawn + IPC channel  | In-process Extism plugin call     |
//! | Ad-hoc permission enforcement   | WASI capability grants per call   |
//!
//! External plugins are compiled-to-WASM modules that must export two functions:
//!
//! * `describe() -> JSON`   — returns [`PackageDescriptor`] as JSON.
//! * `<func_name>(JSON) -> JSON` — one export per plugin function; input is
//!   a JSON-serialised [`RsJsBridgeArgs`], output is [`RsJsBridgeReturns`].

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use extism::{Manifest, Plugin, Wasm};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use proto::sapphillon::v1::{Permission, PermissionType};

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ExtPluginError {
    #[error("extism plugin error: {0}")]
    Extism(#[from] extism::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("function not found in package: {0}")]
    FunctionNotFound(String),

    #[error("permission denied: function '{func}' requires {required:?} but only {granted:?} were granted")]
    PermissionDenied {
        func: String,
        required: Vec<String>,
        granted: Vec<String>,
    },

    #[error("invalid package descriptor: {0}")]
    InvalidDescriptor(String),
}

// ── Package descriptor types (returned by WASM `describe` export) ─────────

/// Full metadata returned by `describe()` in the WASM plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDescriptor {
    pub meta: Meta,
    pub functions: HashMap<String, FunctionSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author_id: String,
    pub package_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSchema {
    /// Permission types (as i32 values matching `PermissionType`) this
    /// function declares it needs.
    pub required_permission_types: Vec<i32>,
    pub description: String,
    pub parameters: Vec<Parameter>,
    pub returns: Vec<ReturnInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub description: String,
    pub r#type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReturnInfo {
    pub name: String,
    pub description: String,
    pub r#type: String,
}

// ── SapphillonPackage ────────────────────────────────────────────────────────

/// A loaded external plugin package backed by a WASM module.
///
/// Call [`SapphillonPackage::from_wasm_path`] to load and validate a WASM
/// plugin — it calls the plugin's `describe` export to extract metadata.
pub struct SapphillonPackage {
    pub meta: Meta,
    pub functions: HashMap<String, FunctionSchema>,
    /// Filesystem path to the `.wasm` file.
    pub wasm_path: String,
}

impl SapphillonPackage {
    /// Load a WASM plugin from `wasm_path` and call its `describe` export to
    /// extract package metadata.
    pub fn from_wasm_path(wasm_path: impl Into<String>) -> Result<Self, ExtPluginError> {
        let wasm_path = wasm_path.into();
        let manifest = Manifest::new([Wasm::file(&wasm_path)]);
        let mut plugin = Plugin::new(&manifest, [], true)?;

        // Call describe() with empty input; output is JSON-encoded PackageDescriptor.
        let raw = plugin.call::<&str, &str>("describe", "")?;
        let descriptor: PackageDescriptor = serde_json::from_str(raw)
            .map_err(|e| ExtPluginError::InvalidDescriptor(e.to_string()))?;

        Ok(Self {
            meta: descriptor.meta,
            functions: descriptor.functions,
            wasm_path,
        })
    }
}

// ── IPC-replacement types ─────────────────────────────────────────────────

/// Call descriptor sent from the Deno bridge op to `extplugin_client`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsJsBridgeArgs {
    pub func_name: String,
    pub args: IndexMap<String, serde_json::Value>,
}

/// Return value produced by the WASM plugin function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsJsBridgeReturns {
    pub args: IndexMap<String, serde_json::Value>,
}

// ── Permission → WASI capability mapping ─────────────────────────────────

fn permissions_to_wasi(
    permissions: &[Permission],
) -> (BTreeMap<PathBuf, PathBuf>, Vec<String>) {
    let mut paths: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut hosts: Vec<String> = Vec::new();

    for perm in permissions {
        match perm.kind() {
            PermissionType::AllowAll => {
                // Broadest grant — give full filesystem and wildcard network.
                paths.insert(PathBuf::from("/"), PathBuf::from("/"));
                hosts.push("*".to_string());
            }
            PermissionType::FilesystemRead | PermissionType::FilesystemWrite => {
                let p = PathBuf::from(&perm.resource);
                paths.insert(p.clone(), p);
            }
            PermissionType::NetworkAccess => {
                hosts.push(perm.resource.clone());
            }
            PermissionType::Unspecified => {}
        }
    }

    (paths, hosts)
}

// ── Permission validation ─────────────────────────────────────────────────

fn check_function_permissions(
    schema: &FunctionSchema,
    granted: &[Permission],
) -> Result<(), ExtPluginError> {
    let granted_types: Vec<i32> = granted.iter().map(|p| p.permission_type).collect();

    // AllowAll in granted satisfies everything.
    if granted_types.contains(&(PermissionType::AllowAll as i32)) {
        return Ok(());
    }

    let missing: Vec<String> = schema
        .required_permission_types
        .iter()
        .filter(|required| !granted_types.contains(required))
        .map(|t| format!("PermissionType({})", t))
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(ExtPluginError::PermissionDenied {
            func: String::new(), // filled in by caller
            required: missing,
            granted: granted_types.iter().map(|t| format!("PermissionType({})", t)).collect(),
        })
    }
}

// ── extplugin_client ──────────────────────────────────────────────────────

/// Call a function inside a WASM plugin via Extism.
///
/// # What this replaces
/// The original used `server_path` + `server_args` to spawn a subprocess and
/// communicate over an IPC channel. Here the WASM sandbox *is* the isolation
/// boundary — Extism's WASI support enforces the filesystem/network limits
/// derived from `permissions` without any inter-process overhead.
///
/// # Contract
/// 1. Validates that `permissions` satisfies the function's declared requirements.
/// 2. Builds an Extism [`Manifest`] with only the WASI capabilities that
///    `permissions` allows.
/// 3. Loads the WASM plugin, calls `func_name`, returns the JSON result.
pub fn extplugin_client(
    package: &SapphillonPackage,
    func_name: &str,
    args: &RsJsBridgeArgs,
    permissions: Vec<Permission>,
) -> Result<RsJsBridgeReturns, ExtPluginError> {
    // 1. Look up the function schema and validate permissions.
    let schema = package
        .functions
        .get(func_name)
        .ok_or_else(|| ExtPluginError::FunctionNotFound(func_name.to_string()))?;

    check_function_permissions(schema, &permissions).map_err(|e| match e {
        ExtPluginError::PermissionDenied { required, granted, .. } => {
            ExtPluginError::PermissionDenied {
                func: func_name.to_string(),
                required,
                granted,
            }
        }
        other => other,
    })?;

    // 2. Build WASI-scoped manifest from permissions.
    let (allowed_paths, allowed_hosts) = permissions_to_wasi(&permissions);

    let mut manifest = Manifest::new([Wasm::file(&package.wasm_path)]);
    if !allowed_paths.is_empty() {
        manifest.allowed_paths = Some(allowed_paths);
    }
    if !allowed_hosts.is_empty() {
        manifest.allowed_hosts = Some(allowed_hosts);
    }

    // 3. Load plugin and call the function.
    let mut plugin = Plugin::new(&manifest, [], true)?;
    let input_json = serde_json::to_string(args)?;
    let output_raw = plugin.call::<&str, &str>(func_name, &input_json)?;
    let returns: RsJsBridgeReturns = serde_json::from_str(output_raw)?;

    Ok(returns)
}
