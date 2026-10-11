use risunest_network_log::{Query, Recorder, Settings, Side};
use serde::Deserialize;
use serde_json::Value;
use std::{io::Write, path::Path, sync::RwLock};
use tauri::{Manager, State};

pub(crate) struct NetworkLogState(RwLock<Result<Recorder, String>>);
impl NetworkLogState {
    fn recorder(&self) -> Result<Recorder, String> { self.0.read().map_err(|_| "network-log-unavailable".to_owned())?.clone() }
    #[cfg(any(mobile, test))]
    pub(crate) fn reopen(&self, paths: &crate::app_paths::AppPaths) -> Result<(), String> {
        let next = initialize(paths).0.into_inner().map_err(|_| "network-log-unavailable")?;
        *self.0.write().map_err(|_| "network-log-unavailable")? = next;
        Ok(())
    }
}

pub(crate) fn initialize(paths: &crate::app_paths::AppPaths) -> NetworkLogState {
    let retained = paths.data.join("network-logs");
    let result = (|| {
        std::fs::create_dir_all(&retained).map_err(|_| "network-log-unavailable".to_owned())?;
        exclude_backup(&retained)?;
        let recorder = Recorder::open(paths.cache.join("network-logs"), retained)?;
        risunest_network_log::install(recorder.clone());
        Ok(recorder)
    })();
    NetworkLogState(RwLock::new(result))
}
fn exclude_backup(path: &Path) -> Result<(), String> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        unsafe extern "C" { fn risunest_network_log_exclude_backup(path: *const std::ffi::c_char) -> bool; }
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| "network-log-unavailable")?;
        if !unsafe { risunest_network_log_exclude_backup(path.as_ptr()) } { return Err("network-log-backup-exclusion-failed".into()); }
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    let _ = path;
    Ok(())
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum Operation {
    Settings,
    Configure { settings: Settings },
    List { query: Query },
    Detail { id: String },
    Body { id: String, side: Side, offset: u64, length: Option<usize>, hex: Option<bool> },
    Events { id: String, offset: u64 },
    ArrivalTimes { id: String, ends: Vec<u64> },
    Copy { id: String, side: Side },
    Retain { id: String },
    Delete { id: String },
    Export { id: String, destination: Option<String> },
}
fn json(value: impl serde::Serialize) -> Result<Value, String> { serde_json::to_value(value).map_err(|_| "network-log-response-failed".into()) }

#[tauri::command]
pub(crate) async fn network_log_command(
    webview: tauri::Webview,
    state: State<'_, NetworkLogState>,
    request: Operation,
) -> Result<Value, String> {
    if webview.label() != "main" { return Err("network-log-access-denied".into()); }
    let recorder = state.recorder()?;
    let app = webview.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let jobs = app.state::<crate::native_file_jobs::NativeFileJobState>();
        let _cleanup_operation = jobs.admit_cleanup_operation().map_err(|_| "network-log-unavailable")?;
        match request {
        Operation::Settings => json(recorder.settings()),
        Operation::Configure { settings } => { recorder.configure(settings)?; json(recorder.settings()) },
        Operation::List { query } => json(recorder.list(query)?),
        Operation::Detail { id } => json(recorder.record(id)?),
        Operation::Body { id, side, offset, length, hex } => json(recorder.snapshot(id)?.body_range(side, offset, length.unwrap_or(risunest_network_log::PAGE_BYTES), hex)?),
        Operation::Events { id, offset } => json(recorder.snapshot(id)?.observations(offset)?),
        Operation::ArrivalTimes { id, ends } => json(recorder.snapshot(id)?.arrival_times(&ends)?),
        Operation::Copy { id, side } => json(recorder.snapshot(id)?.copy_body(side)?),
        Operation::Retain { id } => { recorder.retain(id)?; Ok(Value::Null) },
        Operation::Delete { id } => { recorder.delete(id)?; Ok(Value::Null) },
        Operation::Export { id, destination } => {
            let mut snapshot = recorder.snapshot(id)?;
            if let Some(destination) = destination {
                #[cfg(mobile)]
                { let _ = destination; return Err("network-log-native-picker-required".into()); }
                #[cfg(desktop)]
                {
                    let destination = std::path::PathBuf::from(destination);
                    let parent = destination.parent().ok_or("network-log-invalid-destination")?.canonicalize().map_err(|_| "network-log-invalid-destination")?;
                    let paths = app.state::<crate::app_paths::AppPaths>();
                    if !destination.is_absolute() || paths.owned_roots().iter().any(|root| root.canonicalize().is_ok_and(|root| parent.starts_with(root))) {
                        return Err("network-log-invalid-destination".into());
                    }
                    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| "network-log-export-failed")?;
                    {
                        let mut writer = std::io::BufWriter::new(temporary.as_file_mut());
                        snapshot.export_json(&mut writer)?;
                        writer.flush().map_err(|_| "network-log-export-failed")?;
                    }
                    temporary.as_file().sync_all().map_err(|_| "network-log-export-failed")?;
                    let bytes = temporary.as_file().metadata().map_err(|_| "network-log-export-failed")?.len();
                    temporary.persist(destination).map_err(|_| "network-log-export-failed")?;
                    json(serde_json::json!({ "bytes": bytes, "path": null }))
                }
            } else {
                let state = app.state::<crate::native_file_jobs::NativeFileJobState>();
                let path = crate::native_file_jobs::download_handoff::write_native(&state, |writer| snapshot.export_json(writer))?;
                let bytes = std::fs::metadata(&path).map_err(|_| "network-log-export-failed")?.len();
                json(serde_json::json!({ "path": path, "bytes": bytes }))
            }
        }
    }}).await.map_err(|_| "network-log-worker-failed".to_string())?
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_replaces_the_recorder_without_invalidating_state_references() {
        let root = tempfile::tempdir().unwrap();
        let paths = crate::app_paths::AppPaths::isolated(root.path().join("synthetic-network-log")).unwrap();
        let state = initialize(&paths);
        let borrowed = &state;
        let old = borrowed.recorder().unwrap();
        old.configure(Settings { retain: true, ..Settings::default() }).unwrap();
        old.shutdown();
        state.reopen(&paths).unwrap();
        assert!(old.is_stopped());
        let current = borrowed.recorder().unwrap();
        assert!(!current.is_stopped());
        assert!(!current.settings().enabled);
        assert!(current.settings().retain);
        current.configure(Settings { hide_auth: false, ..Settings::default() }).unwrap();
        assert!(!borrowed.recorder().unwrap().settings().hide_auth);
        current.shutdown();
    }
}
