use std::sync::{Arc, Mutex};
use std::time::Instant;

use proto::sapphillon::v1::{WorkflowCode, WorkflowResult, WorkflowStatus};

use crate::{
    plugin::{
        CorePluginExternalPackage, CorePluginExternalFunction,
        CorePluginPackage, PluginFunctionTrait, PluginIdentifier, PluginPackageTrait,
    },
    permission::PluginFunctionPermissions,
    runtime::{OpStateWorkflowData, run_script},
};

pub struct CoreWorkflowCode {
    pub id: String,
    pub code: String,
    pub plugin_packages: Vec<Arc<dyn PluginPackageTrait>>,
    pub code_revision: i32,
    /// Accumulated results; `run()` always appends, never replaces.
    pub result: Vec<WorkflowResult>,
    pub allowed_permissions: Vec<PluginFunctionPermissions>,
    pub required_permissions: Vec<PluginFunctionPermissions>,
}

impl CoreWorkflowCode {
    pub fn new(
        id: impl Into<String>,
        code: impl Into<String>,
        plugin_packages: Vec<Arc<dyn PluginPackageTrait>>,
        code_revision: i32,
        allowed_permissions: Vec<PluginFunctionPermissions>,
        required_permissions: Vec<PluginFunctionPermissions>,
    ) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            plugin_packages,
            code_revision,
            result: Vec::new(),
            allowed_permissions,
            required_permissions,
        }
    }

    pub fn new_from_proto(
        workflow_code: &WorkflowCode,
        plugin_packages: Vec<Arc<dyn PluginPackageTrait>>,
        required_permissions: Vec<PluginFunctionPermissions>,
        allowed_permissions: Vec<PluginFunctionPermissions>,
    ) -> Self {
        Self::new(
            workflow_code.id.clone(),
            workflow_code.code.clone(),
            plugin_packages,
            workflow_code.code_revision,
            allowed_permissions,
            required_permissions,
        )
    }

    // ── run ───────────────────────────────────────────────────────────

    /// Execute the workflow.  Appends exactly one [`WorkflowResult`].
    /// Never panics — errors are captured as a failed result entry.
    ///
    /// The `_external_plugin_runner_*` parameters are kept for API
    /// compatibility but are no longer used; Extism handles plugin
    /// execution in-process via the WASM sandbox.
    pub fn run(
        &mut self,
        _external_plugin_runner_path: Option<String>,
        _external_plugin_runner_args: Option<Vec<String>>,
    ) {
        let start = Instant::now();

        // ── Collect plugin functions and external packages ────────────
        let mut plugin_fns: Vec<Box<dyn PluginFunctionTrait>> = Vec::new();
        let mut external_packages: Vec<Arc<CorePluginExternalPackage>> = Vec::new();

        for pkg in &self.plugin_packages {
            if pkg.is_external() {
                if let Some(ext) = pkg.as_external_package() {
                    // Rebuild as Arc so runtime can hold it.
                    let arc_pkg = Arc::new(CorePluginExternalPackage {
                        id: ext.id.clone(),
                        name: ext.name.clone(),
                        wasm_path: ext.wasm_path.clone(),
                        functions: ext.functions.iter().map(|f| CorePluginExternalFunction {
                            id: f.id.clone(),
                            name: f.name.clone(),
                            description: f.description.clone(),
                            author_id: f.author_id.clone(),
                            package_id: f.package_id.clone(),
                        }).collect(),
                    });
                    external_packages.push(arc_pkg);
                }
            }
            // Collect all function descriptors (for JS shim generation).
            plugin_fns.extend(pkg.get_functions());
        }

        // ── Build workflow data ───────────────────────────────────────
        let workflow_data = Arc::new(Mutex::new(OpStateWorkflowData::new(
            &self.id,
            true,
            Some(self.allowed_permissions.clone()),
            Some(self.required_permissions.clone()),
            external_packages,
        )));

        let elapsed_ms = || start.elapsed().as_millis() as i64;

        match run_script(&self.code, plugin_fns, Some(Arc::clone(&workflow_data)), None) {
            Ok(data_arc) => {
                let data = data_arc.lock().unwrap();
                self.result.push(WorkflowResult {
                    status: WorkflowStatus::Ok as i32,
                    stdout: data.stdout_to_string(),
                    duration_ms: elapsed_ms(),
                    error_message: String::new(),
                });
            }
            Err(err) => {
                self.result.push(WorkflowResult {
                    status: WorkflowStatus::Error as i32,
                    stdout: String::new(),
                    duration_ms: elapsed_ms(),
                    error_message: err.to_string(),
                });
            }
        }
    }

    // ── extract_used_plugins ──────────────────────────────────────────

    pub fn extract_used_plugins(
        &self,
        available_plugins: &[CorePluginPackage],
    ) -> Vec<PluginIdentifier> {
        let mut found = Vec::new();
        for pkg in available_plugins {
            for func in &pkg.functions {
                let dot     = format!("Sapphillon.{}.{}", pkg.name, func.name);
                let bracket = format!("Sapphillon[\"{}\"][\"{}\"]", pkg.name, func.name);
                if self.code.contains(&dot) || self.code.contains(&bracket) {
                    found.push(PluginIdentifier {
                        package_id: pkg.id.clone(),
                        function_name: func.name.clone(),
                    });
                }
            }
        }
        found
    }
}
