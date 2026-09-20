use super::files::{self, Result};
use std::path::PathBuf;
#[cfg(mobile)]
use tauri::Manager;

#[derive(Clone, Debug)]
pub(super) struct Paths {
    pub data: PathBuf,
    pub roots: Vec<PathBuf>,
    pub control: PathBuf,
}

impl Paths {
    pub fn validate(&self) -> Result<()> {
        files::validate(&self.control)?;
        for path in &self.roots {
            files::validate(path)?;
            if self.control.starts_with(path) || path.starts_with(&self.control) {
                return Err("cleanup-path-overlap".into());
            }
        }
        Ok(())
    }

    pub fn request(&self) -> PathBuf { self.control.join("request.json") }

    pub fn from_app(app: &tauri::AppHandle) -> Result<Self> {
        #[cfg(desktop)]
        { Self::desktop(app.config()) }
        #[cfg(mobile)]
        {
            let data = crate::app_data_root::resolve(app).map_err(|_| "cleanup-path-unavailable")?;
            let cache = app.path().app_cache_dir().map_err(|_| "cleanup-path-unavailable")?;
            let filesystem_data = platform_root(&data)?;
            let cache = platform_root(&cache)?;
            let control = filesystem_data.parent().ok_or("cleanup-path-invalid")?.join("risunest-cleanup");
            let roots = files::independent_roots(vec![filesystem_data, cache]);
            let paths = Self { data, roots, control };
            paths.validate()?;
            Ok(paths)
        }
    }

    #[cfg(desktop)]
    pub fn desktop(config: &tauri::utils::config::Config) -> Result<Self> {
        let id = &config.identifier;
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
            return Err("cleanup-identifier-invalid".into());
        }
        let data_home = dirs::data_dir().ok_or("cleanup-path-unavailable")?;
        let local_home = dirs::data_local_dir().ok_or("cleanup-path-unavailable")?;
        let cache_home = dirs::cache_dir().ok_or("cleanup-path-unavailable")?;
        let config_home = dirs::config_dir().ok_or("cleanup-path-unavailable")?;
        #[cfg(windows)]
        let data = {
            let leaf = config.plugins.0.get("risunest")
                .and_then(|value| value.get("dataDirectoryName"))
                .and_then(serde_json::Value::as_str).unwrap_or(id);
            if leaf.is_empty() || !leaf.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
                return Err("cleanup-identifier-invalid".into());
            }
            data_home.join(leaf)
        };
        #[cfg(not(windows))]
        let data = data_home.join(id);
        let mut roots = vec![data.clone(), data_home.join(id), local_home.join(id), cache_home.join(id), config_home.join(id)];
        #[cfg(windows)]
        roots.push(local_home.join("RisuNestWebViewData"));
        #[cfg(target_os = "linux")]
        roots.push(data_home.join("risunest-webview"));
        #[cfg(target_os = "macos")]
        {
            let library = dirs::home_dir().ok_or("cleanup-path-unavailable")?.join("Library");
            roots.extend([
                library.join("Logs").join(id),
            ]);
        }
        let control = data_home.join(format!("{id}-cleanup"));
        let paths = Self { data, roots: files::independent_roots(roots), control };
        Ok(paths)
    }
}

#[cfg(any(mobile, test))]
fn platform_root(path: &std::path::Path) -> Result<PathBuf> {
    use std::{fs, io};
    if !path.is_absolute() || path.components().any(|part| matches!(part,
        std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return Err("cleanup-path-invalid".into());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if files::linked(&metadata) || !metadata.is_dir() => Err("cleanup-path-redirected".into()),
        Ok(_) => fs::canonicalize(path).map_err(|_| "cleanup-path-unavailable".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("cleanup-path-invalid")?;
            // OS-owned ancestors may use mount aliases; the app-owned root may not.
            let parent = fs::canonicalize(parent).map_err(|_| "cleanup-path-unavailable")?;
            Ok(parent.join(path.file_name().ok_or("cleanup-path-invalid")?))
        },
        Err(_) => Err("cleanup-path-unavailable".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_platform_cache_keeps_its_dedicated_leaf() {
        let parent = tempfile::tempdir().unwrap();
        let cache = parent.path().join("cache");
        assert_eq!(platform_root(&cache).unwrap(), std::fs::canonicalize(parent.path()).unwrap().join("cache"));
        assert!(platform_root(std::path::Path::new("relative")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn trusts_platform_ancestor_alias_but_rejects_redirected_app_root() {
        let parent = tempfile::tempdir().unwrap();
        let real = parent.path().join("real");
        std::fs::create_dir_all(real.join("app")).unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert_eq!(platform_root(&alias.join("app")).unwrap(), std::fs::canonicalize(real.join("app")).unwrap());
        assert!(platform_root(&alias).is_err());
    }
}
