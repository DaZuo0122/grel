//! ELF DT_NEEDED extraction via goblin.

use std::path::Path;

/// Extract all DT_NEEDED library basenames from an ELF binary.
/// Returns an empty Vec if the file is not an ELF or cannot be parsed.
pub fn needed_libs(path: &Path) -> Vec<String> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    match goblin::elf::Elf::parse(&data) {
        Ok(elf) => elf.libraries.iter().map(|lib| (*lib).to_string()).collect(),
        Err(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_elf_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_elf.txt");
        std::fs::write(&path, b"hello world").unwrap();
        assert!(needed_libs(&path).is_empty());
    }

    #[test]
    fn missing_file_returns_empty() {
        assert!(needed_libs(Path::new("/nonexistent/binary")).is_empty());
    }

    #[test]
    fn empty_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        assert!(needed_libs(&path).is_empty());
    }

    #[test]
    fn truncated_elf_magic_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.elf");
        // ELF magic bytes only — not a valid ELF, goblin should fail gracefully
        std::fs::write(&path, b"\x7fELF\x02\x01\x01\x00").unwrap();
        assert!(needed_libs(&path).is_empty());
    }

    /// Smoke-test that parsing a real ELF binary does not panic.
    /// The binary may or may not have DT_NEEDED entries; we only assert
    /// the call completes without crashing.
    #[cfg(target_os = "linux")]
    #[test]
    fn real_elf_binary_parses_without_panic() {
        // /bin/ls is available on virtually every Linux installation
        let path = Path::new("/bin/ls");
        if path.exists() {
            let _ = needed_libs(path);
        }
    }
}
