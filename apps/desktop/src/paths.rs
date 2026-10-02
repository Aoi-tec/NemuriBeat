use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
pub struct DataPaths {
    pub root: PathBuf,
    pub personal: PathBuf,
    pub cache: PathBuf,
}
/// Explicit path wins, otherwise OS data root (Windows LocalAppData), then executable directory.
pub fn resolve(
    explicit: Option<PathBuf>,
    os_data: Option<PathBuf>,
    executable: &Path,
    demo: bool,
) -> Result<DataPaths> {
    let root = match explicit {
        Some(path) => {
            probe(&path)?;
            path
        }
        None => {
            let preferred = os_data.map(|p| p.join("NemuriBeat"));
            match preferred {
                Some(p) if probe(&p).is_ok() => p,
                _ => {
                    let p = executable
                        .parent()
                        .context("Executable has no parent")?
                        .join("NemuriBeatData");
                    probe(&p)?;
                    p
                }
            }
        }
    };
    let scope = if demo {
        root.join("Demo")
    } else {
        root.clone()
    };
    let personal = scope.join("PersonalData/nemuribeat.sqlite");
    let cache = scope.join("Cache/derived.sqlite");
    std::fs::create_dir_all(personal.parent().unwrap())?;
    std::fs::create_dir_all(cache.parent().unwrap())?;
    Ok(DataPaths {
        root,
        personal,
        cache,
    })
}
fn probe(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    let test = path.join(format!(".write-check-{}", uuid::Uuid::new_v4()));
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&test)?;
    drop(f);
    std::fs::remove_file(test)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_permanent_cache_and_demo() {
        let root = std::env::temp_dir().join(format!("hbt-paths-{}", uuid::Uuid::new_v4()));
        let p = resolve(Some(root.clone()), None, Path::new("app.exe"), false).unwrap();
        let d = resolve(Some(root.clone()), None, Path::new("app.exe"), true).unwrap();
        assert!(p.personal.ends_with("PersonalData/nemuribeat.sqlite"));
        assert!(p.cache.ends_with("Cache/derived.sqlite"));
        assert_ne!(p.personal, d.personal);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_os_root_falls_back_beside_executable() {
        let root = std::env::temp_dir().join(format!("hbt-fallback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let p = resolve(None, None, &root.join("app.exe"), false).unwrap();
        assert_eq!(p.root, root.join("NemuriBeatData"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
