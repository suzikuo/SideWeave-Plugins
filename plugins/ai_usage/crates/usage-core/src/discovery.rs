use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static DISCOVERY_HOME: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

struct HomeGuard(Option<PathBuf>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        DISCOVERY_HOME.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Run one adapter discovery pass as if `root` were the user's home directory.
/// The override is thread-local and always restored, including when discovery panics.
pub fn with_discovery_home<T>(root: &Path, discover: impl FnOnce() -> T) -> T {
    let previous = DISCOVERY_HOME.with(|slot| slot.borrow_mut().replace(root.to_path_buf()));
    let _guard = HomeGuard(previous);
    discover()
}

/// The active external home root, if this call is part of a configured scan.
pub fn discovery_home_root() -> Option<PathBuf> {
    DISCOVERY_HOME.with(|slot| slot.borrow().clone())
}

pub fn home_dir() -> Option<PathBuf> {
    discovery_home_root().or_else(dirs::home_dir)
}

/// Linux-style directories under an added home root. Outside a root scan these
/// keep the host platform's usual `dirs` behavior.
pub fn config_dir() -> Option<PathBuf> {
    discovery_home_root().map(|home| home.join(".config")).or_else(dirs::config_dir)
}

pub fn data_dir() -> Option<PathBuf> {
    discovery_home_root().map(|home| home.join(".local").join("share")).or_else(dirs::data_dir)
}

pub fn data_local_dir() -> Option<PathBuf> {
    discovery_home_root().map(|home| home.join(".local").join("share")).or_else(dirs::data_local_dir)
}

pub fn cache_dir() -> Option<PathBuf> {
    discovery_home_root().map(|home| home.join(".cache")).or_else(dirs::cache_dir)
}
