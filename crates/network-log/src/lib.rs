mod decoding;
mod export;
mod masking;
mod model;
#[cfg(feature = "reqwest")]
pub mod reqwest;
mod storage;
pub use model::*;
pub use storage::Snapshot;

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex, OnceLock, RwLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const QUEUE_BYTES: usize = 4 * 1024 * 1024;
const QUEUE_EVENTS: usize = 512;
const ACTIVE_LIMIT: usize = 64;
const CHUNK_BYTES: usize = 64 * 1024;
static GLOBAL: OnceLock<RwLock<Option<Recorder>>> = OnceLock::new();

#[derive(Clone)]
pub struct Recorder(Arc<Shared>);
struct Shared {
    sender: SyncSender<Event>,
    settings: Mutex<Settings>,
    enabled: AtomicBool,
    queued: AtomicUsize,
    active: AtomicUsize,
    warning: Mutex<Option<String>>,
    stopping: AtomicBool,
    stopped: AtomicBool,
}
struct Signal {
    id: String,
    owner: Arc<Shared>,
    start: Instant,
    hide: bool,
    limit: u64,
    websocket: bool,
    pending: AtomicUsize,
    outcome: AtomicU8,
    reason: AtomicU8,
    body_reason: [AtomicU8; 2],
    response: Mutex<Option<Message>>,
    representations: Mutex<[Option<&'static str>; 2]>,
    dirty: AtomicBool,
    elapsed: AtomicU64,
    observed: [AtomicU64; 2],
    admitted: [AtomicU64; 2],
    binary: [AtomicBool; 2],
    partial: AtomicBool,
    limited: [AtomicBool; 2],
}
impl Signal {
    fn stop(&self, reason: u8) {
        self.dirty.store(true, Ordering::Release);
        let _ = self
            .reason
            .compare_exchange(0, reason, Ordering::AcqRel, Ordering::Relaxed);
        if reason == 2 {
            *self.owner.warning.lock().unwrap() = Some("queue-full".into());
        }
    }
}
impl Signal {
    fn stop_body(&self, side: Side, reason: u8) {
        let _ = self.body_reason[side.index()].compare_exchange(
            0,
            reason,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        self.dirty.store(true, Ordering::Release);
        if reason == 2 {
            *self.owner.warning.lock().unwrap() = Some("queue-full".into());
        }
    }
    fn capture_reason(&self) -> u8 {
        [
            self.reason.load(Ordering::Relaxed),
            self.body_reason[0].load(Ordering::Relaxed),
            self.body_reason[1].load(Ordering::Relaxed),
        ]
        .into_iter()
        .find(|r| *r != 0)
        .unwrap_or(0)
    }
}
impl Drop for Signal {
    fn drop(&mut self) {
        self.owner.active.fetch_sub(1, Ordering::Relaxed);
    }
}
#[derive(Clone, Default)]
pub struct Capture(Option<Arc<Signal>>);
pub struct CaptureGuard(pub Capture);
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.0.finish("incomplete");
    }
}
impl std::ops::Deref for CaptureGuard {
    type Target = Capture;
    fn deref(&self) -> &Capture {
        &self.0
    }
}

struct Envelope {
    signal: Arc<Signal>,
    bytes: usize,
    event: Payload,
}
impl Drop for Envelope {
    fn drop(&mut self) {
        self.signal
            .owner
            .queued
            .fetch_sub(self.bytes, Ordering::Relaxed);
        self.signal.pending.fetch_sub(1, Ordering::Release);
    }
}
enum Payload {
    Start(Record),
    Bytes(Side, Vec<u8>, u64, Option<String>),
}
enum Event {
    Wake,
    Capture(Envelope),
    Job(Box<dyn FnOnce(&mut storage::Store) + Send>),
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn binary(headers: &http::HeaderMap) -> bool {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    !content_type.is_empty()
        && !content_type.starts_with("text/")
        && !["json", "xml", "javascript", "x-www-form-urlencoded"]
            .iter()
            .any(|s| content_type.contains(s))
}
pub fn install(recorder: Recorder) {
    *GLOBAL.get_or_init(Default::default).write().unwrap() = Some(recorder);
}
pub fn global() -> Option<Recorder> {
    GLOBAL.get().and_then(|g| g.read().unwrap().clone())
}
pub fn begin(source: &str, method: &str, url: &str, headers: &http::HeaderMap) -> CaptureGuard {
    CaptureGuard(
        global()
            .map(|r| r.begin(source, method, url, headers))
            .unwrap_or_default(),
    )
}
pub fn enabled() -> bool {
    global().is_some_and(|r| r.0.enabled.load(Ordering::Relaxed))
}

impl Recorder {
    pub fn open(recent: PathBuf, retained: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_EVENTS);
        let mut store = storage::Store::open(recent, retained)?;
        let settings = store.settings()?;
        let shared = Arc::new(Shared {
            sender,
            enabled: AtomicBool::new(settings.enabled),
            settings: Mutex::new(settings),
            queued: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            warning: Mutex::new(None),
            stopping: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        });
        let owner = shared.clone();
        std::thread::Builder::new()
            .name("network-log".into())
            .spawn(move || {
                let mut active = std::collections::HashMap::<String, (Arc<Signal>, Record)>::new();
                loop {
                    if owner.stopping.load(Ordering::Acquire) {
                        for (signal, _) in active.values() {
                            signal.stop(1);
                        }
                        flush(&mut store, &mut active, 2);
                        let _ = store.clear_recent();
                        break;
                    }
                    let next = if active.is_empty() {
                        receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    } else {
                        receiver.recv_timeout(std::time::Duration::from_millis(50))
                    };
                    let event = match next {
                        Ok(event) => event,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            flush(&mut store, &mut active, 0);
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    match event {
                        Event::Wake => {}
                        Event::Capture(envelope) => {
                            let signal = &envelope.signal;
                            let settings = owner.settings.lock().unwrap().clone();
                            let result = match &envelope.event {
                                Payload::Start(record) => {
                                    let result = store.insert(record, &settings);
                                    if result.is_ok() {
                                        let mut record = record.clone();
                                        store.prepare_decoding(
                                            &mut record,
                                            Side::Request,
                                            signal.limit,
                                        );
                                        active.insert(record.id.clone(), (signal.clone(), record));
                                    }
                                    result
                                }
                                Payload::Bytes(side, bytes, elapsed, kind) => {
                                    if let Some((_, record)) = active.get_mut(&signal.id) {
                                        apply_metadata(&mut store, signal, record, &settings);
                                        signal.dirty.store(true, Ordering::Release);
                                        if [3, 4, 6].contains(
                                            &signal.body_reason[side.index()]
                                                .load(Ordering::Relaxed),
                                        ) {
                                            Ok(())
                                        } else {
                                            store.append(
                                                record,
                                                *side,
                                                bytes,
                                                *elapsed,
                                                kind.as_deref(),
                                                &settings,
                                            )
                                        }
                                    } else {
                                        Ok(())
                                    }
                                }
                            };
                            if let Err(error) = result {
                                let reason = match error.as_str() {
                                    "quota" => 3,
                                    "decode-failed" => 6,
                                    _ => 4,
                                };
                                if let Payload::Bytes(side, ..) = &envelope.event {
                                    signal.stop_body(*side, reason);
                                } else {
                                    signal.stop(reason);
                                }
                                *owner.warning.lock().unwrap() = Some(error);
                            }
                        }
                        Event::Job(job) => {
                            flush(&mut store, &mut active, 1);
                            job(&mut store);
                            // Promotion updates the writer's copy before accepting more chunks.
                            for (id, (_, record)) in &mut active {
                                if let Ok(saved) = store.record(id) {
                                    record.retained = saved.retained;
                                }
                            }
                        }
                    }
                    flush(&mut store, &mut active, 0);
                }
                drop(store);
                owner.stopped.store(true, Ordering::Release);
            })
            .map_err(|_| "network-log-unavailable".to_string())?;
        Ok(Self(shared))
    }
    pub fn settings(&self) -> Settings {
        self.0.settings.lock().unwrap().clone()
    }
    pub fn configure(&self, settings: Settings) -> Result<(), String> {
        settings.validate()?;
        let next = settings.clone();
        self.job(move |store| store.configure(&next))?;
        if !settings.enabled {
            self.0.enabled.store(false, Ordering::Release);
        }
        *self.0.settings.lock().unwrap() = settings.clone();
        self.0.enabled.store(settings.enabled, Ordering::Release);
        *self.0.warning.lock().unwrap() = None;
        if !settings.enabled {
            self.job(|store| {
                store.stop_active = true;
                Ok(())
            })?;
        }
        Ok(())
    }
    pub fn begin(
        &self,
        source: &str,
        method: &str,
        url: &str,
        headers: &http::HeaderMap,
    ) -> Capture {
        if !self.0.enabled.load(Ordering::Relaxed) {
            return Capture::default();
        }
        if self
            .0
            .active
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < ACTIVE_LIMIT).then_some(n + 1)
            })
            .is_err()
        {
            *self.0.warning.lock().unwrap() = Some("concurrent-limit".into());
            return Capture::default();
        }
        let settings = self.settings();
        let signal = Arc::new(Signal {
            id: uuid::Uuid::new_v4().to_string(),
            owner: self.0.clone(),
            start: Instant::now(),
            hide: settings.hide_auth,
            limit: settings.body_mib * MIB,
            websocket: method == "WS",
            pending: AtomicUsize::new(0),
            outcome: AtomicU8::new(0),
            reason: AtomicU8::new(0),
            body_reason: [AtomicU8::new(0), AtomicU8::new(0)],
            response: Mutex::new(None),
            representations: Mutex::new([None, None]),
            dirty: AtomicBool::new(true),
            elapsed: AtomicU64::new(0),
            observed: [AtomicU64::new(0), AtomicU64::new(0)],
            admitted: [AtomicU64::new(0), AtomicU64::new(0)],
            binary: [AtomicBool::new(binary(headers)), AtomicBool::new(false)],
            partial: AtomicBool::new(false),
            limited: [AtomicBool::new(false), AtomicBool::new(false)],
        });
        let capture = Capture(Some(signal.clone()));
        let headers = match masking::headers(headers, settings.hide_auth) {
            Ok(h) => h,
            Err(_) => {
                signal.stop(5);
                vec![]
            }
        };
        if url.len() > 8192 || source.len() > 80 || method.len() > 32 {
            signal.stop(5);
        }
        let shorten = |s: &str, n| s.chars().take(n).collect::<String>();
        let record = Record {
            id: signal.id.clone(),
            source: shorten(source, 80),
            started_at: now(),
            elapsed_ms: 0,
            retained: settings.retain,
            header_masking: settings.hide_auth,
            state: "active".into(),
            capture_error: None,
            request: Message {
                method: Some(shorten(method, 32)),
                url: shorten(url, 8192),
                headers,
                body: Body {
                    binary: signal.binary[0].load(Ordering::Relaxed),
                    representation: "observed".into(),
                    ..Body::default()
                },
                status: None,
            },
            response: Message::default(),
            stream_kind: (method == "WS").then(|| "websocket".into()),
            event_bytes: 0,
        };
        let bytes = record.request.queued_bytes() + record.source.capacity() + 2048;
        capture.enqueue(Payload::Start(record), bytes);
        capture
    }
    fn job<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut storage::Store) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        if self.0.stopping.load(Ordering::Acquire) {
            return Err("network-log-unavailable".into());
        }
        let (tx, rx) = mpsc::channel();
        self.0
            .sender
            .send(Event::Job(Box::new(move |store| {
                let _ = tx.send(job(store));
            })))
            .map_err(|_| "network-log-unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "network-log-unavailable".to_string())?
    }
    pub fn list(&self, query: Query) -> Result<Listing, String> {
        let mut list = self.job(move |store| store.list(&query))?;
        list.warning = self.0.warning.lock().unwrap().clone().or(list.warning);
        Ok(list)
    }
    pub fn record(&self, id: String) -> Result<Record, String> {
        self.job(move |s| s.record(&id))
    }
    pub fn snapshot(&self, id: String) -> Result<Snapshot, String> {
        self.job(move |s| s.snapshot(&id))
    }
    pub fn retain(&self, id: String) -> Result<(), String> {
        let settings = self.settings();
        self.job(move |s| s.retain(&id, &settings))
    }
    pub fn delete(&self, id: String) -> Result<(), String> {
        self.job(move |s| s.delete(&id))?;
        *self.0.warning.lock().unwrap() = None;
        Ok(())
    }
    pub fn shutdown(&self) {
        self.0.enabled.store(false, Ordering::Release);
        self.0.stopping.store(true, Ordering::Release);
        let _ = self.0.sender.try_send(Event::Wake);
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !self.0.stopped.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.0.stopped.load(Ordering::Acquire)
    }
}
fn apply_metadata(
    store: &mut storage::Store,
    signal: &Signal,
    record: &mut Record,
    settings: &Settings,
) {
    if let Some(response) = signal.response.lock().unwrap().take() {
        record.response = response;
        store.prepare_decoding(record, Side::Response, signal.limit);
        if record.response.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("content-type") && h.value.contains("text/event-stream")
        }) {
            record.stream_kind = Some("sse".into());
        }
        if let Err(error) = store.admit_metadata(record, settings) {
            record.response.headers.clear();
            signal.stop_body(Side::Response, if error == "quota" { 3 } else { 4 });
            *signal.owner.warning.lock().unwrap() = Some(error);
        }
        signal.dirty.store(true, Ordering::Release);
    }
    for (index, value) in signal
        .representations
        .lock()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        if let Some(value) = value.take() {
            record
                .body_mut(if index == 0 {
                    Side::Request
                } else {
                    Side::Response
                })
                .representation = value.into();
            signal.dirty.store(true, Ordering::Release);
        }
    }
}
fn flush(
    store: &mut storage::Store,
    active: &mut std::collections::HashMap<String, (Arc<Signal>, Record)>,
    force: u8,
) {
    let stop = std::mem::take(&mut store.stop_active);
    active.retain(|_, (signal, record)| {
        if stop {
            signal.stop(1);
        }
        let outcome = signal.outcome.load(Ordering::Acquire);
        let settings = signal.owner.settings.lock().unwrap().clone();
        apply_metadata(store, signal, record, &settings);
        let terminal = (outcome != 0 || signal.reason.load(Ordering::Relaxed) == 1)
            && signal.pending.load(Ordering::Acquire) == 0;
        if force == 0
            && !terminal
            && (signal.start.elapsed().as_millis() as u64) < record.elapsed_ms + 250
        {
            return true;
        }
        if !terminal && force != 2 && !signal.dirty.swap(false, Ordering::AcqRel) {
            return true;
        }
        record.elapsed_ms = if outcome == 0 {
            signal.start.elapsed().as_millis() as u64
        } else {
            signal.elapsed.load(Ordering::Relaxed).saturating_sub(1)
        };
        for side in [Side::Request, Side::Response] {
            let body = record.body_mut(side);
            if !matches!(
                body.representation.as_str(),
                "decoded-gzip" | "decoded-deflate"
            ) {
                body.observed_bytes = signal.observed[side.index()].load(Ordering::Relaxed);
            }
            body.truncated = body.captured_bytes < body.observed_bytes;
            body.binary = signal.binary[side.index()].load(Ordering::Relaxed);
        }
        if terminal || force == 2 {
            record.state = match outcome {
                1 => "complete",
                2 => "error",
                _ => "incomplete",
            }
            .into();
        }
        if record.request.body.truncated || record.response.body.truncated {
            for side in [Side::Request, Side::Response] {
                if matches!(
                    record.body(side).representation.as_str(),
                    "decoded-gzip" | "decoded-deflate"
                ) && record.body(side).truncated
                {
                    signal.limited[side.index()].store(true, Ordering::Relaxed);
                }
            }
        }
        if terminal
            && outcome == 1
            && signal.capture_reason() == 0
            && !store.decoding_complete(&record.id)
        {
            signal.stop(6);
        }
        record.capture_error = match signal.capture_reason() {
            1 => Some("stopped"),
            2 => Some("queue-full"),
            3 => Some("quota"),
            4 => Some("write-failed"),
            5 => Some("metadata-limit"),
            6 => Some("decode-failed"),
            _ if signal.limited.iter().any(|l| l.load(Ordering::Relaxed)) => Some("body-limit"),
            _ if signal.partial.load(Ordering::Relaxed) => Some("started-late"),
            _ => None,
        }
        .map(str::to_owned);
        if let Err(error) = store.update(record) {
            signal.stop(4);
            *signal.owner.warning.lock().unwrap() = Some(error);
        }
        if terminal || force == 2 {
            store.finish(&record.id);
        }
        !terminal && force != 2
    });
}
impl Capture {
    pub fn is_active(&self) -> bool {
        self.0.as_ref().is_some_and(|s| {
            matches!(s.reason.load(Ordering::Relaxed), 0 | 5)
                && s.owner.enabled.load(Ordering::Relaxed)
        })
    }
    pub fn was_stopped(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|s| s.reason.load(Ordering::Relaxed) == 1)
    }
    pub fn guard(&self) -> CaptureGuard {
        CaptureGuard(self.clone())
    }
    fn enqueue(&self, event: Payload, bytes: usize) {
        let Some(signal) = &self.0 else {
            return;
        };
        let side = match &event {
            Payload::Bytes(side, ..) => Some(*side),
            _ => None,
        };
        let failed = || {
            if let Some(side) = side {
                signal.stop_body(side, 2);
            } else {
                signal.stop(2);
            }
        };
        if signal
            .owner
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                n.checked_add(bytes).filter(|n| *n <= QUEUE_BYTES)
            })
            .is_err()
        {
            failed();
            return;
        }
        signal.pending.fetch_add(1, Ordering::Relaxed);
        let envelope = Envelope {
            signal: signal.clone(),
            bytes,
            event,
        };
        if signal
            .owner
            .sender
            .try_send(Event::Capture(envelope))
            .is_err()
        {
            failed();
        }
    }
    pub fn response(&self, status: u16, url: &str, headers: &http::HeaderMap) {
        let Some(signal) = &self.0 else {
            return;
        };
        if !self.is_active() {
            return;
        }
        signal.binary[1].store(binary(headers), Ordering::Relaxed);
        let headers = match masking::headers(headers, signal.hide) {
            Ok(h) => h,
            Err(_) => {
                signal.stop(5);
                vec![]
            }
        };
        let response = Message {
            status: Some(status),
            url: url.chars().take(8192).collect(),
            headers,
            body: Body {
                binary: signal.binary[1].load(Ordering::Relaxed),
                representation: "observed".into(),
                ..Body::default()
            },
            method: None,
        };
        // One bounded response slot per admitted capture survives a full body queue.
        *signal.response.lock().unwrap() = Some(response);
        signal.dirty.store(true, Ordering::Release);
        let _ = signal.owner.sender.try_send(Event::Wake);
    }
    pub fn mark_partial(&self) {
        if let Some(signal) = &self.0 {
            signal.partial.store(true, Ordering::Relaxed);
            signal.dirty.store(true, Ordering::Release);
        }
    }
    pub fn representation(&self, side: Side, value: &'static str) {
        if self.is_active() {
            if value.starts_with("encoded-") {
                if let Some(signal) = &self.0 {
                    signal.binary[side.index()].store(true, Ordering::Relaxed);
                }
            }
            if let Some(signal) = &self.0 {
                signal.representations.lock().unwrap()[side.index()] = Some(value);
                signal.dirty.store(true, Ordering::Release);
            }
        }
    }
    pub fn append(&self, side: Side, bytes: &[u8]) {
        self.append_message(side, bytes, None);
    }
    pub fn append_message(&self, side: Side, bytes: &[u8], kind: Option<&str>) {
        let Some(signal) = &self.0 else {
            return;
        };
        if !signal.owner.enabled.load(Ordering::Relaxed) {
            signal.stop(1);
            return;
        }
        signal.observed[side.index()].fetch_add(bytes.len() as u64, Ordering::Relaxed);
        signal.dirty.store(true, Ordering::Release);
        if !self.is_active() || signal.body_reason[side.index()].load(Ordering::Relaxed) != 0 {
            return;
        }
        if kind == Some("binary")
            || (matches!(kind, Some("ping" | "pong")) && std::str::from_utf8(bytes).is_err())
        {
            signal.binary[side.index()].store(true, Ordering::Relaxed);
        }
        let limit = if !signal.websocket && signal.binary[side.index()].load(Ordering::Relaxed) {
            BINARY_BYTES
        } else {
            signal.limit
        };
        let prior = signal.admitted[side.index()].fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let count =
            bytes
                .len()
                .min(limit.saturating_sub(prior) as usize)
                .min(if kind == Some("binary") {
                    BINARY_BYTES as usize
                } else {
                    usize::MAX
                });
        if bytes.is_empty() && kind.is_some() {
            self.enqueue(
                Payload::Bytes(
                    side,
                    vec![],
                    signal.start.elapsed().as_millis() as u64,
                    kind.map(str::to_owned),
                ),
                128,
            );
        }
        for chunk in bytes[..count].chunks(CHUNK_BYTES) {
            if !self.is_active() || signal.body_reason[side.index()].load(Ordering::Relaxed) != 0 {
                break;
            }
            self.enqueue(
                Payload::Bytes(
                    side,
                    chunk.to_vec(),
                    signal.start.elapsed().as_millis() as u64,
                    kind.map(|k| k.chars().take(32).collect()),
                ),
                chunk.len() + 128,
            );
        }
        if count < bytes.len() && !signal.binary[side.index()].load(Ordering::Relaxed) {
            signal.limited[side.index()].store(true, Ordering::Relaxed);
        }
    }
    pub fn finish(&self, state: &str) {
        if let Some(signal) = &self.0 {
            if signal
                .elapsed
                .compare_exchange(
                    0,
                    (signal.start.elapsed().as_millis() as u64).saturating_add(1),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                signal.outcome.store(
                    match state {
                        "complete" => 1,
                        "error" => 2,
                        _ => 3,
                    },
                    Ordering::Release,
                );
                signal.dirty.store(true, Ordering::Release);
                let _ = signal.owner.sender.try_send(Event::Wake);
            }
        }
    }
}
#[cfg(test)]
mod tests;
