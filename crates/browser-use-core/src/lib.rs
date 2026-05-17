//! Core agent contracts for browser-use-rs.

/// Version of the upstream browser-use source that this crate initially targets.
pub const INITIAL_UPSTREAM_COMMIT: &str = "933e28c599ddd74c15a48568f159da95547e40dd";

#[cfg(test)]
mod tests {
    use super::INITIAL_UPSTREAM_COMMIT;

    #[test]
    fn target_commit_is_pinned() {
        assert_eq!(INITIAL_UPSTREAM_COMMIT.len(), 40);
    }
}
