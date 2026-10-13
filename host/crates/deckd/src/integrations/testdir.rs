//! A throwaway directory under `$TMPDIR`, removed on drop.
//!
//! Lives in the library rather than in `tests/` because the installer's own
//! unit tests and `tests/integrations.rs` must create temp `CLAUDE_CONFIG_DIR`
//! / `CODEX_HOME` trees the same way. Nothing in the daemon calls it; the
//! rule it exists to keep is contract §9: **a test never touches the user's
//! `~/.claude`, `~/.codex` or `~/Library/LaunchAgents`.**
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub fn new(tag: &str) -> TestDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "openwalkietalkie-integrations-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the test directory");
        TestDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A sub-directory that exists.
    pub fn sub(&self, name: &str) -> PathBuf {
        let path = self.path.join(name);
        std::fs::create_dir_all(&path).expect("create the sub-directory");
        path
    }

    /// Write a file, creating its parents.
    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent directory");
        }
        std::fs::write(&path, contents).expect("write the test file");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
