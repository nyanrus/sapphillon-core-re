//! Canonical serializable types for Sapphillon.
//!
//! Types are defined with `prost::Message` derive so they can be encoded/decoded
//! in protobuf wire format without requiring a separate `.proto` file or code
//! generation step at build time.

pub mod sapphillon {
    pub mod v1 {
        use prost::Message;
        use serde::{Deserialize, Serialize};

        // ── Permission ────────────────────────────────────────────────────

        /// The category of a permission grant.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[repr(i32)]
        pub enum PermissionType {
            Unspecified  = 0,
            /// Grants everything; collapses all other grants.
            AllowAll     = 1,
            FilesystemRead  = 4,
            FilesystemWrite = 5,
            NetworkAccess   = 6,
        }

        impl PermissionType {
            pub fn from_i32(v: i32) -> Self {
                match v {
                    1 => Self::AllowAll,
                    4 => Self::FilesystemRead,
                    5 => Self::FilesystemWrite,
                    6 => Self::NetworkAccess,
                    _ => Self::Unspecified,
                }
            }
        }

        /// A single permission grant. `resource` carries the path or URL
        /// relevant to the type (empty for `AllowAll`).
        #[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
        pub struct Permission {
            /// Numeric value of [`PermissionType`].
            #[prost(int32, tag = "1")]
            pub permission_type: i32,

            /// Path (for filesystem) or origin/URL (for network).
            #[prost(string, tag = "2")]
            pub resource: String,
        }

        impl Permission {
            pub fn new(permission_type: PermissionType, resource: impl Into<String>) -> Self {
                Self {
                    permission_type: permission_type as i32,
                    resource: resource.into(),
                }
            }

            pub fn kind(&self) -> PermissionType {
                PermissionType::from_i32(self.permission_type)
            }
        }

        // ── Workflow ──────────────────────────────────────────────────────

        #[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
        pub struct WorkflowCode {
            #[prost(string, tag = "1")]
            pub id: String,

            #[prost(string, tag = "2")]
            pub code: String,

            #[prost(int32, tag = "3")]
            pub code_revision: i32,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum WorkflowStatus {
            Ok    = 0,
            Error = 1,
        }

        #[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
        pub struct WorkflowResult {
            /// Numeric value of [`WorkflowStatus`].
            #[prost(int32, tag = "1")]
            pub status: i32,

            /// Captured stdout joined into a single string.
            #[prost(string, tag = "2")]
            pub stdout: String,

            /// Wall-clock execution time in milliseconds.
            #[prost(int64, tag = "3")]
            pub duration_ms: i64,

            /// Human-readable error message, empty on success.
            #[prost(string, tag = "4")]
            pub error_message: String,
        }

        // ── Plugin metadata (lightweight; full logic lives in sapphillon_core) ─

        #[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
        pub struct PluginPackage {
            #[prost(string, tag = "1")]
            pub id: String,

            #[prost(string, tag = "2")]
            pub name: String,
        }

        #[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
        pub struct PluginFunction {
            #[prost(string, tag = "1")]
            pub id: String,

            #[prost(string, tag = "2")]
            pub name: String,

            #[prost(string, tag = "3")]
            pub description: String,
        }
    }
}
