use super::{Result, SyncError};
use crate::persistent_store::{commands::with_store_mut, PersistentStore};
use std::collections::BTreeSet;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use tauri::{AppHandle, Manager};
#[derive(Default)]
pub(crate) struct ServerSyncCommandState {
    cleanup_closed: AtomicBool,
    running: AtomicBool,
    cancelled: Mutex<Arc<AtomicBool>>,
}
struct Running<'a>(&'a ServerSyncCommandState);
impl Drop for Running<'_> { fn drop(&mut self) { self.0.running.store(false, Ordering::Release); } }
impl ServerSyncCommandState {
    pub(crate) fn begin_cleanup(&self) -> Result<()> { self.cleanup_closed.store(true, Ordering::Release); self.cancel() }
    pub(crate) fn cleanup_drained(&self) -> Result<bool> { Ok(!self.running.load(Ordering::Acquire)) }
    pub(crate) fn finish_cleanup(&self) -> Result<()> { if !self.cleanup_drained()? { return Err(SyncError::new("server-sync-busy",409)); } self.cleanup_closed.store(false,Ordering::Release); Ok(()) }
    pub(crate) fn cancel(&self) -> Result<()> { self.cancelled.lock().map_err(|_| SyncError::new("server-sync-state-unavailable",503))?.store(true,Ordering::Release); Ok(()) }
    fn claim(&self) -> Result<Running<'_>> {
        self.running.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_| SyncError::new("server-sync-busy",409))?;
        if self.cleanup_closed.load(Ordering::Acquire) { self.running.store(false,Ordering::Release); return Err(SyncError::new("cleanup-pending",409)); }
        Ok(Running(self))
    }
    fn claim_preparation(&self) -> Result<(Running<'_>, Arc<AtomicBool>)> {
        let running=self.claim()?; let flag=Arc::new(AtomicBool::new(false));
        *self.cancelled.lock().map_err(|_| SyncError::new("server-sync-state-unavailable",503))?=flag.clone(); Ok((running,flag))
    }
}
fn claim_library<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<crate::native_file_jobs::admission::Permit> {
    app.state::<crate::native_file_jobs::NativeFileJobState>()
        .admission
        .server()
        .map_err(|code| SyncError::new(code, 409))
}
fn job_store<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PersistentStore> {
    with_store_mut(app.state(), |store| store.open_native_job_store())
        .map_err(|error| SyncError::caused("local-store-unavailable", 503, error.to_string()))
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| SyncError::new("server-sync-worker-unavailable", 503))?
}
async fn logged_blocking<T: Send + 'static>(
    stage: &str,
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    blocking(operation).await.map_err(|error| { crate::nlog!("warn", "Server {stage} operation failed: {}", error.code); error })
}
#[tauri::command]
pub(crate) async fn server_sync_asset_status(
    app: AppHandle,
) -> Result<crate::persistent_store::asset_residency::ResidencyStatus> {
    logged_blocking("asset-status", move || job_store(&app)?.asset_residency_status()).await
}
#[tauri::command]
pub(crate) async fn server_sync_asset_policy(
    app: AppHandle,
    policy: super::residency::AssetPolicy,
) -> Result<crate::persistent_store::asset_residency::ResidencyStatus> {
    logged_blocking("asset-policy", move || server_sync_asset_policy_operation(&app, policy)).await
}

fn server_sync_asset_policy_operation<R: tauri::Runtime>(app: &AppHandle<R>, policy: super::residency::AssetPolicy) -> Result<crate::persistent_store::asset_residency::ResidencyStatus> {
    let _admission = claim_library(&app)?;
    let state = app.state::<ServerSyncCommandState>();
    let (_running, cancelled) = state.claim_preparation()?;
    job_store(&app)?.asset_residency_set_policy_cancelled(policy, Some(cancelled.clone()), || {
        if cancelled.load(Ordering::Acquire) {
            Err(SyncError::new("cancelled", 409))
        } else {
            Ok(())
        }
    })
}
#[tauri::command]
pub(crate) async fn server_sync_asset_evict(
    app: AppHandle,
) -> Result<crate::persistent_store::asset_residency::ResidencyStatus> {
    logged_blocking("asset-evict", move || server_sync_asset_evict_operation(&app)).await
}

fn server_sync_asset_evict_operation<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<crate::persistent_store::asset_residency::ResidencyStatus> {
    let _admission = claim_library(&app)?;
    let state = app.state::<ServerSyncCommandState>();
    let (_running, cancelled) = state.claim_preparation()?;
    job_store(&app)?.asset_residency_evict_cancelled(Some(cancelled.clone()), || {
        if cancelled.load(Ordering::Acquire) {
            Err(SyncError::new("cancelled", 409))
        } else {
            Ok(())
        }
    })
}
#[tauri::command]
pub(crate) async fn server_sync_cache_usage(
    app: AppHandle,
) -> Result<super::management::CacheUsage> {
    logged_blocking("cache-usage", move || manage_cache(&app, false)).await
}
#[tauri::command]
pub(crate) async fn server_sync_cache_cleanup(
    app: AppHandle,
) -> Result<super::management::CacheUsage> {
    logged_blocking("cache-cleanup", move || manage_cache(&app, true)).await
}
fn manage_cache(app: &AppHandle, clean: bool) -> Result<super::management::CacheUsage> {
    let state=app.state::<ServerSyncCommandState>();
    let _admission=if clean { Some(claim_library(app)?) } else { None };
    let _running=if clean { Some(state.claim()?) } else { None };
    let store=job_store(app)?;
    let blocked=if !clean && state.running.load(Ordering::Acquire) { Some("server-sync-busy") } else { None };
    let active=store.server_stored_config()?.map(|config| risunest_sync_wire::hash(format!("{}:{}",config.library_id,config.device_id).as_bytes()));
    super::management::cache_usage(store.repository_root(),active.as_deref(),&BTreeSet::new(),blocked,clean)
}
