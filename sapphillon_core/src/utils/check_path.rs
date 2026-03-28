use std::path::{Path, PathBuf};

/// Normalize a path: resolve `.` and `..` lexically, strip trailing slashes.
fn normalize(p: impl AsRef<Path>) -> PathBuf {
    let mut out = PathBuf::new();
    for component in p.as_ref().components() {
        use std::path::Component::*;
        match component {
            CurDir => {}
            ParentDir => { out.pop(); }
            c => out.push(c),
        }
    }
    out
}

fn is_ancestor_or_equal(ancestor: &Path, descendant: &Path) -> bool {
    let a = normalize(ancestor);
    let d = normalize(descendant);
    d.starts_with(&a)
}

/// Returns `true` if every path in `b` is covered by at least one path in `a`
/// via the ancestor relationship (`a_entry` is a prefix of `b_entry`).
///
/// * Empty `b` → always `true`.
/// * Non-empty `b`, empty `a` → always `false`.
pub fn paths_cover_by_ancestor<A, B>(a: &[A], b: &[B]) -> bool
where
    A: AsRef<Path>,
    B: AsRef<Path>,
{
    if b.is_empty() { return true; }
    if a.is_empty() { return false; }

    b.iter().all(|b_entry| {
        a.iter().any(|a_entry| is_ancestor_or_equal(a_entry.as_ref(), b_entry.as_ref()))
    })
}

/// Returns `true` if `a` and `b` cover the same set of paths — every element
/// of each is an ancestor-of or equal-to some element in the other.
pub fn paths_cover_as_set<A, B>(a: &[A], b: &[B]) -> bool
where
    A: AsRef<Path>,
    B: AsRef<Path>,
{
    paths_cover_by_ancestor(a, b) && paths_cover_by_ancestor(b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_covers_child() {
        assert!(paths_cover_by_ancestor(&["/foo"], &["/foo/bar/baz"]));
    }

    #[test]
    fn sibling_does_not_cover() {
        assert!(!paths_cover_by_ancestor(&["/foo/other"], &["/foo/bar"]));
    }

    #[test]
    fn empty_b_always_true() {
        let empty: &[&str] = &[];
        assert!(paths_cover_by_ancestor(&["/foo"], empty));
        assert!(paths_cover_by_ancestor(empty, empty));
    }

    #[test]
    fn empty_a_nonempty_b_false() {
        let empty: &[&str] = &[];
        assert!(!paths_cover_by_ancestor(empty, &["/foo"]));
    }

    #[test]
    fn dot_dot_normalized() {
        assert!(paths_cover_by_ancestor(&["/foo"], &["/foo/bar/../baz"]));
    }
}
