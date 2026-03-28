use url::Url;

/// Returns `true` if URL `a` covers URL `b`:
/// same scheme + host + port, and `a`'s path is a prefix of `b`'s path.
fn url_covers(a: &str, b: &str) -> bool {
    let (Ok(ua), Ok(ub)) = (Url::parse(a), Url::parse(b)) else {
        return false;
    };

    // Scheme, host, and port must match.
    if ua.scheme() != ub.scheme() { return false; }
    if ua.host_str() != ub.host_str() { return false; }
    if ua.port_or_known_default() != ub.port_or_known_default() { return false; }

    // `a`'s path must be a prefix of `b`'s path (segment-boundary aware).
    let a_path = ua.path().trim_end_matches('/');
    let b_path = ub.path();
    b_path.starts_with(a_path)
        && (a_path.is_empty()
            || b_path.len() == a_path.len()
            || b_path.as_bytes().get(a_path.len()) == Some(&b'/'))
}

/// Returns `true` if every URL in `b` is covered by at least one URL in `a`
/// via origin + path-prefix matching.
///
/// * Empty `b` → always `true`.
/// * Non-empty `b`, empty `a` → always `false`.
/// * Malformed URLs in `b` are never covered.
pub fn urls_cover_by_ancestor<A, B>(a: &[A], b: &[B]) -> bool
where
    A: AsRef<str>,
    B: AsRef<str>,
{
    if b.is_empty() { return true; }
    if a.is_empty() { return false; }

    b.iter().all(|b_entry| {
        a.iter().any(|a_entry| url_covers(a_entry.as_ref(), b_entry.as_ref()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_path_prefix() {
        assert!(urls_cover_by_ancestor(
            &["https://example.com/api"],
            &["https://example.com/api/v1/users"],
        ));
    }

    #[test]
    fn different_host_not_covered() {
        assert!(!urls_cover_by_ancestor(
            &["https://example.com/api"],
            &["https://other.com/api"],
        ));
    }

    #[test]
    fn prefix_must_be_segment_boundary() {
        // /api does not cover /apiv2 (no slash boundary)
        assert!(!urls_cover_by_ancestor(
            &["https://example.com/api"],
            &["https://example.com/apiv2"],
        ));
    }

    #[test]
    fn malformed_not_covered() {
        assert!(!urls_cover_by_ancestor(
            &["https://example.com"],
            &["not a url"],
        ));
    }

    #[test]
    fn empty_b_always_true() {
        let empty: &[&str] = &[];
        assert!(urls_cover_by_ancestor(&["https://example.com"], empty));
    }
}
