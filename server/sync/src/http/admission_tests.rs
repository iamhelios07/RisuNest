use super::*;
use tokio::sync::{mpsc, watch};

struct Fixture {
    _root: tempfile::TempDir,
    app: App,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    endpoint: String,
    library: String,
    token: String,
    entered: mpsc::UnboundedReceiver<()>,
    release: Arc<Semaphore>,
}

impl Drop for Fixture {
    fn drop(&mut self) { self.task.abort(); }
}

impl Fixture {
    async fn new(slots: usize, buffers: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::init(root.path()).unwrap());
        let device = store.add_device().unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let app = App {
            store,
            slots: Arc::new(Semaphore::new(slots)),
            wait_slots: Arc::new(Semaphore::new(8)),
            buffers: Arc::new(Semaphore::new(buffers)),
            materializers: Arc::new(Semaphore::new(1)),
            media_slots: Arc::new(Semaphore::new(16)),
            notice_slots: Arc::new(Semaphore::new(32)),
            workload: Workload::new(),
            shutdown: receiver,
            _lifetime: Arc::new(()),
        };
        let (sender, entered) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        let held = {
            let release = release.clone();
            move || {
                let release = release.clone();
                let sender = sender.clone();
                async move {
                    sender.send(()).unwrap();
                    release.acquire().await.unwrap().forget();
                    StatusCode::NO_CONTENT
                }
            }
        };
        let router = Router::new()
            .route("/held", get(held.clone()))
            .route("/uploads/frames", post(held))
            .route("/head", get(head))
            .route_layer(middleware::from_fn_with_state(app.clone(), authorize))
            .with_state(app.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { _root: root, app, shutdown, task, endpoint, library: device.library_id, token: device.token, entered, release }
    }

    fn request(&self, path: &str) -> tokio::task::JoinHandle<reqwest::Response> {
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
        let request = if path == "/uploads/frames" { client.post(format!("{}{path}", self.endpoint)) }
            else { client.get(format!("{}{path}", self.endpoint)) };
        let request = request.bearer_auth(&self.token).header("x-risu-library", &self.library);
        tokio::spawn(async move { request.send().await.unwrap() })
    }

    async fn entered(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.recv()).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn one_device_can_receive_while_two_upload_requests_are_active() {
    let mut fixture = Fixture::new(8, 4).await;
    let first = fixture.request("/uploads/frames");
    let second = fixture.request("/uploads/frames");
    fixture.entered().await;
    fixture.entered().await;
    let response = fixture.request("/head").await.unwrap();
    fixture.release.add_permits(2);
    assert_eq!(first.await.unwrap().status(), 204);
    assert_eq!(second.await.unwrap().status(), 204);
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn exhausted_request_and_buffer_capacity_waits_then_completes() {
    for (slots, buffers, path) in [(1, 4, "/held"), (8, 1, "/uploads/frames")] {
        let mut fixture = Fixture::new(slots, buffers).await;
        let first = fixture.request(path);
        fixture.entered().await;
        let mut second = fixture.request(path);
        let early = tokio::time::timeout(Duration::from_millis(200), &mut second).await;
        fixture.release.add_permits(2);
        assert_eq!(first.await.unwrap().status(), 204);
        assert!(early.is_err(), "capacity exhaustion must wait instead of returning a refusal");
        assert_eq!(second.await.unwrap().status(), 204);
    }
}

#[tokio::test]
async fn shutdown_releases_queued_requests_without_cancelling_active_work() {
    let mut fixture = Fixture::new(1, 4).await;
    let active = fixture.request("/held");
    fixture.entered().await;
    let mut queued = fixture.request("/head");
    assert!(tokio::time::timeout(Duration::from_millis(200), &mut queued).await.is_err());
    assert_eq!(fixture.app.workload.status().unwrap().active_requests, 1);
    fixture.shutdown.send_replace(true);
    let response = queued.await.unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(response.json::<serde_json::Value>().await.unwrap()["error"], "server-updating");
    fixture.release.add_permits(1);
    assert_eq!(active.await.unwrap().status(), 204);
}

#[tokio::test]
async fn cancelled_capacity_waits_release_their_queue_position() {
    let fixture = Fixture::new(8, 4).await;
    for capacity in [&fixture.app.slots, &fixture.app.buffers, &fixture.app.media_slots, &fixture.app.notice_slots] {
        let count = capacity.available_permits();
        let held = capacity.acquire_many(count as u32).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), fixture.app.wait_slot(capacity.clone())).await.is_err());
        drop(held);
        let permit = tokio::time::timeout(Duration::from_secs(1), fixture.app.wait_slot(capacity.clone())).await.unwrap().unwrap();
        drop(permit);
        assert_eq!(capacity.available_permits(), count);
    }
}

#[tokio::test]
async fn a_registration_revoked_while_queued_cannot_run_after_admission() {
    let mut fixture = Fixture::new(1, 4).await;
    let active = fixture.request("/held");
    fixture.entered().await;
    let mut queued = fixture.request("/head");
    assert!(tokio::time::timeout(Duration::from_millis(200), &mut queued).await.is_err());
    let device = fixture.app.store.authenticate(&fixture.library, &fixture.token).unwrap();
    fixture.app.store.revoke_device(&device.id).unwrap();
    fixture.release.add_permits(1);
    assert_eq!(active.await.unwrap().status(), 204);
    assert_eq!(queued.await.unwrap().status(), 401);
}
