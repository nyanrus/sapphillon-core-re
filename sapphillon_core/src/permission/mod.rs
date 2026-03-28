use proto::sapphillon::v1::{Permission, PermissionType};
use crate::utils::{check_path::paths_cover_by_ancestor, check_url::urls_cover_by_ancestor};

// ── Permissions ───────────────────────────────────────────────────────────

/// A set of [`Permission`] grants attached to a workflow or plugin function.
#[derive(Debug, Clone, Default)]
pub struct Permissions {
    pub permissions: Vec<Permission>,
}

impl Permissions {
    pub fn new(permissions: Vec<Permission>) -> Self {
        Self { permissions }
    }

    /// Deduplicate and normalize:
    /// * If `AllowAll` is present, collapse to exactly `[AllowAll]`.
    /// * For filesystem and network entries, drop entries that are subsumed by
    ///   a more general entry already in the list.
    pub fn merge(self) -> Self {
        let perms = self.permissions;

        // AllowAll wins over everything.
        if perms.iter().any(|p| p.kind() == PermissionType::AllowAll) {
            return Self::new(vec![Permission::new(PermissionType::AllowAll, "")]);
        }

        // Deduplicate: for each permission type group, keep only the minimal
        // covering set (ancestors subsume descendants).
        let mut out: Vec<Permission> = Vec::new();

        for kind in [
            PermissionType::FilesystemRead,
            PermissionType::FilesystemWrite,
            PermissionType::NetworkAccess,
        ] {
            let group: Vec<&Permission> = perms
                .iter()
                .filter(|p| p.kind() == kind)
                .collect();

            match kind {
                PermissionType::FilesystemRead | PermissionType::FilesystemWrite => {
                    let resources: Vec<&str> =
                        group.iter().map(|p| p.resource.as_str()).collect();
                    for p in &group {
                        let others: Vec<&str> = resources
                            .iter()
                            .filter(|&&r| r != p.resource.as_str())
                            .copied()
                            .collect();
                        // Keep this entry only if no other entry subsumes it.
                        if !paths_cover_by_ancestor(&others, &[p.resource.as_str()]) {
                            out.push((*p).clone());
                        }
                    }
                }
                PermissionType::NetworkAccess => {
                    let resources: Vec<&str> =
                        group.iter().map(|p| p.resource.as_str()).collect();
                    for p in &group {
                        let others: Vec<&str> = resources
                            .iter()
                            .filter(|&&r| r != p.resource.as_str())
                            .copied()
                            .collect();
                        if !urls_cover_by_ancestor(&others, &[p.resource.as_str()]) {
                            out.push((*p).clone());
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        // Pass-through for any other custom permission types.
        for p in &perms {
            match p.kind() {
                PermissionType::FilesystemRead
                | PermissionType::FilesystemWrite
                | PermissionType::NetworkAccess
                | PermissionType::AllowAll
                | PermissionType::Unspecified => {}
                _ => out.push(p.clone()),
            }
        }

        Self::new(out)
    }
}

// ── PluginFunctionPermissions ─────────────────────────────────────────────

/// Associates a [`Permissions`] set with a specific plugin function ID.
#[derive(Debug, Clone)]
pub struct PluginFunctionPermissions {
    pub plugin_function_id: String,
    pub permissions: Permissions,
}

// ── CheckPermissionResult ─────────────────────────────────────────────────

#[derive(Debug)]
pub enum CheckPermissionResult {
    Ok,
    /// Carries exactly which permissions were required but not present.
    MissingPermission(Permissions),
}

// ── check_permission ──────────────────────────────────────────────────────

/// Returns `Ok` if every permission in `required` is covered by `granted`.
///
/// Coverage rules:
/// * `AllowAll` in `granted` covers everything.
/// * `FilesystemRead`/`Write`: granted paths must be ancestors of required paths.
/// * `NetworkAccess`: granted origins must cover required origins.
/// * All other types: matching `PermissionType` value in granted is sufficient.
pub fn check_permission(
    granted: &Permissions,
    required: &Permissions,
) -> CheckPermissionResult {
    // AllowAll shortcut.
    if granted
        .permissions
        .iter()
        .any(|p| p.kind() == PermissionType::AllowAll)
    {
        return CheckPermissionResult::Ok;
    }

    let mut missing: Vec<Permission> = Vec::new();

    for req in &required.permissions {
        let covered = match req.kind() {
            PermissionType::FilesystemRead | PermissionType::FilesystemWrite => {
                let granted_paths: Vec<&str> = granted
                    .permissions
                    .iter()
                    .filter(|p| p.kind() == req.kind())
                    .map(|p| p.resource.as_str())
                    .collect();
                paths_cover_by_ancestor(&granted_paths, &[req.resource.as_str()])
            }
            PermissionType::NetworkAccess => {
                let granted_urls: Vec<&str> = granted
                    .permissions
                    .iter()
                    .filter(|p| p.kind() == PermissionType::NetworkAccess)
                    .map(|p| p.resource.as_str())
                    .collect();
                urls_cover_by_ancestor(&granted_urls, &[req.resource.as_str()])
            }
            _ => granted
                .permissions
                .iter()
                .any(|p| p.permission_type == req.permission_type),
        };

        if !covered {
            missing.push(req.clone());
        }
    }

    if missing.is_empty() {
        CheckPermissionResult::Ok
    } else {
        CheckPermissionResult::MissingPermission(Permissions::new(missing))
    }
}

// ── find_allowed_permissions ──────────────────────────────────────────────

/// Finds and merges all permissions from `allowed` whose
/// `plugin_function_id` matches any of `target_function_ids`.
pub fn find_allowed_permissions(
    allowed: &[PluginFunctionPermissions],
    target_function_ids: &[&str],
) -> Permissions {
    let merged: Vec<Permission> = allowed
        .iter()
        .filter(|pfp| target_function_ids.contains(&pfp.plugin_function_id.as_str()))
        .flat_map(|pfp| pfp.permissions.permissions.iter().cloned())
        .collect();

    Permissions::new(merged).merge()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::sapphillon::v1::PermissionType;

    fn fs_read(path: &str) -> Permission {
        Permission::new(PermissionType::FilesystemRead, path)
    }
    fn net(url: &str) -> Permission {
        Permission::new(PermissionType::NetworkAccess, url)
    }
    fn allow_all() -> Permission {
        Permission::new(PermissionType::AllowAll, "")
    }

    #[test]
    fn allow_all_covers_everything() {
        let granted = Permissions::new(vec![allow_all()]);
        let required = Permissions::new(vec![fs_read("/secret"), net("https://evil.com")]);
        assert!(matches!(check_permission(&granted, &required), CheckPermissionResult::Ok));
    }

    #[test]
    fn ancestor_path_covers_child() {
        let granted = Permissions::new(vec![fs_read("/data")]);
        let required = Permissions::new(vec![fs_read("/data/file.txt")]);
        assert!(matches!(check_permission(&granted, &required), CheckPermissionResult::Ok));
    }

    #[test]
    fn sibling_path_missing() {
        let granted = Permissions::new(vec![fs_read("/data/a")]);
        let required = Permissions::new(vec![fs_read("/data/b/file.txt")]);
        assert!(matches!(
            check_permission(&granted, &required),
            CheckPermissionResult::MissingPermission(_)
        ));
    }

    #[test]
    fn merge_collapses_allow_all() {
        let p = Permissions::new(vec![fs_read("/tmp"), allow_all()]).merge();
        assert_eq!(p.permissions.len(), 1);
        assert_eq!(p.permissions[0].kind(), PermissionType::AllowAll);
    }
}
