use super::{Result, ServerConfig};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::Duration,
};

type DeviceKey = (String, String);
static DEVICES: OnceLock<Mutex<BTreeMap<DeviceKey, Weak<RequestSlots>>>> = OnceLock::new();

#[derive(Default)]
struct State {
    active: usize,
    waiting: VecDeque<Arc<()>>,
}

#[derive(Default)]
pub(super) struct RequestSlots {
    state: Mutex<State>,
    changed: Condvar,
}

pub(super) struct Permit<'a>(&'a RequestSlots);

impl RequestSlots {
    pub(super) fn for_device(config: &ServerConfig) -> Arc<Self> {
        let mut devices = DEVICES.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|_, slots| slots.strong_count() > 0);
        let key = (config.library_id.clone(), config.device_id.clone());
        if let Some(slots) = devices.get(&key).and_then(Weak::upgrade) {
            return slots;
        }
        let slots = Arc::new(Self::default());
        devices.insert(key, Arc::downgrade(&slots));
        slots
    }

    pub(super) fn acquire(&self, active: impl Fn() -> Result<()>) -> Result<Permit<'_>> {
        let ticket = Arc::new(());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.waiting.push_back(ticket.clone());
        self.changed.notify_all();
        loop {
            if let Err(error) = active() {
                state.waiting.retain(|queued| !Arc::ptr_eq(queued, &ticket));
                self.changed.notify_all();
                return Err(error);
            }
            if state.active < 2 && state.waiting.front().is_some_and(|queued| Arc::ptr_eq(queued, &ticket)) {
                state.waiting.pop_front();
                state.active += 1;
                self.changed.notify_all();
                return Ok(Permit(self));
            }
            // Cancellation is an atomic flag shared with native jobs.
            state = self.changed.wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::SyncError;
    use std::sync::{atomic::{AtomicBool, Ordering}, mpsc};
    use std::time::Instant;

    fn waiters(slots: &RequestSlots, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = slots.state.lock().unwrap();
        while state.waiting.len() != count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "request did not enter the queue");
            state = slots.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    #[test]
    fn requests_share_two_slots_and_wait_in_arrival_order() {
        let slots = Arc::new(RequestSlots::default());
        let first = slots.acquire(|| Ok(())).unwrap();
        let second = slots.acquire(|| Ok(())).unwrap();
        let (sent, received) = mpsc::channel();
        std::thread::scope(|scope| {
            for id in 0..3 {
                let worker_slots = slots.clone();
                let sent = sent.clone();
                scope.spawn(move || {
                    let _permit = worker_slots.acquire(|| Ok(())).unwrap();
                    sent.send(id).unwrap();
                });
                waiters(&slots, id + 1);
            }
            assert!(received.try_recv().is_err());
            drop(first);
            for id in 0..3 {
                assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), id);
            }
        });
        drop(second);
        assert_eq!(slots.state.lock().unwrap().active, 0);
    }

    #[test]
    fn a_cancelled_waiter_does_not_block_later_requests() {
        let slots = Arc::new(RequestSlots::default());
        let first = slots.acquire(|| Ok(())).unwrap();
        let second = slots.acquire(|| Ok(())).unwrap();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let job = scope.spawn(|| slots.acquire(|| {
                if cancelled.load(Ordering::Acquire) { Err(SyncError::new("cancelled", 409)) } else { Ok(()) }
            }).err().unwrap());
            waiters(&slots, 1);
            cancelled.store(true, Ordering::Release);
            assert_eq!(job.join().unwrap().code, "cancelled");
        });
        assert!(slots.state.lock().unwrap().waiting.is_empty());
        drop(first);
        let _later = slots.acquire(|| Ok(())).unwrap();
        drop(second);
    }

    #[test]
    fn a_device_keeps_its_slots_when_its_endpoint_changes() {
        let mut config = super::super::tests::config("http://127.0.0.1:1");
        config.device_id = uuid::Uuid::new_v4().to_string();
        let original = RequestSlots::for_device(&config);
        config.endpoint = "http://127.0.0.1:2".into();
        assert!(Arc::ptr_eq(&original, &RequestSlots::for_device(&config)));
        config.device_id = "different-device".into();
        assert!(!Arc::ptr_eq(&original, &RequestSlots::for_device(&config)));
    }

    #[test]
    fn independent_clients_hold_slots_until_the_response_body_finishes() {
        use axum::{body::{Body, Bytes}, routing::get, Router};
        use futures::StreamExt;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (release, gate) = tokio::sync::watch::channel(false);
        let (entered, requests) = mpsc::channel();
        let router = Router::new().route("/held", get(move || {
            let mut gate = gate.clone();
            entered.send(()).unwrap();
            async move {
                let prefix = futures::stream::once(async { Ok::<_, std::io::Error>(Bytes::from_static(b"o")) });
                let suffix = futures::stream::once(async move {
                    gate.wait_for(|released| *released).await.unwrap();
                    Ok::<_, std::io::Error>(Bytes::from_static(b"k"))
                });
                Body::from_stream(prefix.chain(suffix))
            }
        }));
        let server = runtime.spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut config = super::super::tests::config(&endpoint);
        config.device_id = uuid::Uuid::new_v4().to_string();
        let clients = (0..3).map(|_| super::super::ServerClient::new(config.clone()).unwrap()).collect::<Vec<_>>();
        let slots = clients[0].slots.clone();
        std::thread::scope(|scope| {
            let mut jobs = Vec::new();
            for (index, client) in clients.iter().enumerate() {
                jobs.push(scope.spawn(move || client.request(reqwest::Method::GET, "held", &[], None, &[], 1024)));
                if index < 2 { requests.recv_timeout(Duration::from_secs(5)).unwrap(); }
            }
            waiters(&slots, 1);
            assert!(requests.try_recv().is_err());
            release.send_replace(true);
            for job in jobs {
                let reply = job.join().unwrap().unwrap();
                assert_eq!(reply.status, 200);
                assert_eq!(reply.body, b"ok");
            }
        });
        assert_eq!(slots.state.lock().unwrap().active, 0);
        server.abort();
    }
}
