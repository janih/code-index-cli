//! Gitignore matching helpers.
//!
//! The `ignore` crate matches literal rules only: a `dir/` rule matches the
//! directory itself but not files under it (in real git, traversal stops at
//! the ignored directory). This helper bridges the difference by checking
//! every ancestor directory, so files under ignored directories are filtered
//! the way git's traversal would.

use std::path::Path;

/// Returns true when the relative path (or any of its ancestor directories)
/// is ignored by `matcher`.
pub fn is_ignored(matcher: &ignore::gitignore::Gitignore, relative: &Path, is_dir: bool) -> bool {
    if matcher.matched(relative, is_dir).is_ignore() {
        return true;
    }
    for ancestor in relative.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        if matcher.matched(ancestor, true).is_ignore() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use ignore::gitignore::GitignoreBuilder;

    fn matcher(rules: &str) -> ignore::gitignore::Gitignore {
        let mut builder = GitignoreBuilder::new("/ws");
        for line in rules.lines() {
            builder.add_line(None, line).unwrap();
        }
        builder.build().unwrap()
    }

    #[test]
    fn directory_rules_cover_descendants() {
        let m = matcher("secrets/\n");
        assert!(is_ignored(&m, Path::new("secrets"), true));
        assert!(is_ignored(&m, Path::new("secrets/key.pem"), false));
        assert!(is_ignored(&m, Path::new("secrets/deep/key.pem"), false));
        assert!(!is_ignored(&m, Path::new("src/main.rs"), false));
    }

    #[test]
    fn glob_rules_match_files_directly() {
        let m = matcher("*.snap\n");
        assert!(is_ignored(&m, Path::new("x.snap"), false));
        assert!(is_ignored(&m, Path::new("a/b/y.snap"), false));
    }

    #[test]
    fn negation_of_parent_still_matches_children_rules() {
        let m = matcher("target\n");
        // bare name matches dirs AND files anywhere
        assert!(is_ignored(&m, Path::new("target"), true));
        assert!(is_ignored(&m, Path::new("crates/a/target"), true));
        assert!(is_ignored(&m, Path::new("target/out.rs"), false));
    }
}
