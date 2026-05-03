/// Utilities for determining whether a rigg file is managed by the user.
///
/// A file is considered user-managed if its first line (trimmed of leading
/// whitespace) starts with the marker `# rigg:managed-by-user`.  This lets
/// operators take over individual files without changing the global
/// `rigg.ownership` config key.
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

/// The hand-takeover marker that Quelch looks for on the first line of any
/// rigg file it is about to overwrite.
const MARKER: &str = "# rigg:managed-by-user";

/// Returns `true` if the file at `path` exists and its first line (after
/// stripping leading whitespace) starts with [`MARKER`].
///
/// If the file does not exist, or if any I/O error occurs while reading it,
/// the function returns `false` — callers should treat a read failure the
/// same as "not marked".
pub fn is_managed_by_user(path: &Path) -> bool {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut reader = BufReader::new(file);
    let mut first_line = String::new();

    match reader.read_line(&mut first_line) {
        Ok(0) => false, // empty file
        Ok(_) => first_line.trim_start().starts_with(MARKER),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_file(contents: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    #[test]
    fn is_managed_by_user_recognises_marker() {
        // First line is the exact marker.
        let f = write_file("# rigg:managed-by-user\nsome: yaml\n");
        assert!(is_managed_by_user(f.path()));

        // First line is something else.
        let f = write_file("# foo\nsome: yaml\n");
        assert!(!is_managed_by_user(f.path()));

        // File does not exist.
        let p = std::path::Path::new("/tmp/__quelch_does_not_exist_xyz.yaml");
        assert!(!is_managed_by_user(p));
    }

    #[test]
    fn is_managed_by_user_handles_leading_whitespace() {
        let f = write_file("   # rigg:managed-by-user\nsome: yaml\n");
        assert!(is_managed_by_user(f.path()));

        let f = write_file("\t# rigg:managed-by-user\nsome: yaml\n");
        assert!(is_managed_by_user(f.path()));
    }

    #[test]
    fn is_managed_by_user_returns_false_for_missing_file() {
        let p = std::path::Path::new("/tmp/__quelch_ownership_missing_test.yaml");
        assert!(!is_managed_by_user(p));
    }

    #[test]
    fn is_managed_by_user_returns_false_for_empty_file() {
        let f = write_file("");
        assert!(!is_managed_by_user(f.path()));
    }

    #[test]
    fn is_managed_by_user_requires_marker_on_first_line() {
        // Marker is present but NOT on the first line.
        let f = write_file("name: foo\n# rigg:managed-by-user\n");
        assert!(!is_managed_by_user(f.path()));
    }
}
