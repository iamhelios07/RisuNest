use super::*;
use std::{fs, time::Duration};

struct Fixture {
    root: tempfile::TempDir,
    recorder: Recorder,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let recorder =
            Recorder::open(root.path().join("recent"), root.path().join("kept")).unwrap();
        Self { root, recorder }
    }
    fn enable(&self, retain: bool, hide: bool) {
        self.recorder
            .configure(Settings {
                enabled: true,
                retain,
                hide_auth: hide,
                ..Settings::default()
            })
            .unwrap();
    }
    fn begin(&self) -> CaptureGuard {
        self.recorder
            .begin(
                "synthetic",
                "POST",
                "https://synthetic.example/path?key=query-remains",
                &http::HeaderMap::new(),
            )
            .guard()
    }
    fn detail(&self, capture: &Capture) -> Record {
        self.recorder
            .record(capture.0.as_ref().unwrap().id.clone())
            .unwrap()
    }
    fn snapshot(&self, capture: &Capture) -> Snapshot {
        self.recorder
            .snapshot(capture.0.as_ref().unwrap().id.clone())
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.recorder.shutdown();
    }
}
fn text_headers() -> http::HeaderMap {
    let mut h = http::HeaderMap::new();
    h.insert("content-type", "text/event-stream".parse().unwrap());
    h
}

#[test]
fn defaults_are_opt_in_and_disabled_capture_does_not_copy_or_enqueue() {
    let f = Fixture::new();
    assert!(!f.recorder.settings().enabled);
    assert!(f.recorder.settings().hide_auth);
    let capture = f.begin();
    capture.append(Side::Response, &[7; 65536]);
    capture.finish("complete");
    assert!(capture.0 .0.is_none());
    assert_eq!(f.recorder.0.queued.load(Ordering::Relaxed), 0);
    assert!(f.recorder.list(Query::default()).unwrap().items.is_empty());
}
#[test]
fn finite_auth_masking_preserves_duplicates_and_does_not_scrub_other_content() {
    let f = Fixture::new();
    f.enable(false, true);
    let secret = "sk_synthetic_only_abcdefghijklmnopqrstuvwxyz7vQ9m2";
    let mut headers = text_headers();
    headers.append("authorization", format!("Bearer {secret}").parse().unwrap());
    headers.append(
        "authorization",
        "Basic synthetic-credentials".parse().unwrap(),
    );
    headers.insert("x-unknown-key", "unchanged-unknown-secret".parse().unwrap());
    headers.insert("cookie", "session=synthetic".parse().unwrap());
    let c = f
        .recorder
        .begin(
            "test",
            "POST",
            "https://synthetic.example/?token=visible",
            &headers,
        )
        .guard();
    c.append(Side::Request, b"body-secret-remains");
    c.response(401, "https://synthetic.example/?token=visible", &headers);
    c.append(Side::Response, b"error-body-remains");
    c.finish("complete");
    let record = f.detail(&c);
    assert_eq!(
        record
            .request
            .headers
            .iter()
            .filter(|h| h.name == "authorization")
            .count(),
        2
    );
    assert!(record
        .request
        .headers
        .iter()
        .any(|h| h.value == "Bearer sk••••••7vQ9m2"));
    assert!(record
        .request
        .headers
        .iter()
        .any(|h| h.value == "Basic [REDACTED]"));
    let mut output = Vec::new();
    f.snapshot(&c).export_json(&mut output).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["request"]["body"]["data"], "body-secret-remains");
    assert_eq!(json["response"]["body"]["data"], "error-body-remains");
    assert!(!String::from_utf8(output).unwrap().contains(secret));
    f.enable(false, false);
    assert_eq!(
        f.detail(&c).request.headers[0].value,
        record.request.headers[0].value
    );
    let unhidden = f
        .recorder
        .begin("test", "GET", "https://synthetic.example", &headers)
        .guard();
    unhidden.finish("complete");
    assert!(f
        .detail(&unhidden)
        .request
        .headers
        .iter()
        .any(|h| h.value == format!("Bearer {secret}")));
}
#[test]
fn known_header_rules_are_deterministic_and_short_or_structured_tokens_are_hidden() {
    for name in ["X-Api-Key", "api-key", "X-Goog-API-KEY"] {
        assert_eq!(masking::mask(name, b"short"), Some("[REDACTED]".into()));
        assert_eq!(
            masking::mask(name, b"abcdefghijklmnop"),
            Some("••••••mnop".into())
        );
        assert_eq!(
            masking::mask(name, b"abcdefghijklmnopqrstuvwxyz123456"),
            Some("ab••••••123456".into())
        );
        assert_eq!(
            masking::mask(name, b"ey.synthetic.jwt-token-more-than-32"),
            Some("[REDACTED]".into())
        );
        assert_eq!(
            masking::mask(name, b"ghp_synthetic-abcdefghijklmnopqrstuvwxyz"),
            Some("[REDACTED]".into())
        );
    }
    for name in ["Cookie", "Set-Cookie", "X-Amz-Security-Token"] {
        assert_eq!(
            masking::mask(name, b"abcdefghijklmnopqrstuvwxyz123456"),
            Some("[REDACTED]".into())
        );
    }
    assert_eq!(
        masking::mask("Proxy-Authorization", b"Token abcdefghijklmnop"),
        Some("Token ••••••mnop".into())
    );
    assert_eq!(
        masking::mask("Authorization", b"Digest key=value"),
        Some("Digest [REDACTED]".into())
    );
    assert_eq!(masking::mask("x-unknown", b"visible"), None);
}
#[test]
fn sse_raw_bytes_split_unicode_and_partial_tail_round_trip_through_json() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    c.response(200, "https://synthetic.example/final", &text_headers());
    let text = ": heartbeat\r\nevent: token\r\nid: 5\r\ndata: 한글😀\r\ndata: \"x\"\\\t\r\n\r\ndata: [DONE]\n\nunknown: preserve\rdata: unfinished";
    for bytes in text.as_bytes().chunks(7) {
        c.append(Side::Response, bytes);
    }
    c.finish("error");
    let mut snapshot = f.snapshot(&c);
    assert_eq!(snapshot.record.state, "error");
    assert_eq!(snapshot.body_page(Side::Response, 0).unwrap().text, text);
    let mut output = Vec::new();
    f.snapshot(&c).export_json(&mut output).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["response"]["body"]["data"], text);
    assert_eq!(json["stream"]["kind"], "sse");
    assert!(json["stream"]["observations"].as_array().unwrap().len() > 1);
}
#[test]
fn stopped_captures_do_not_resume_or_cancel_the_transport() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    c.append(Side::Response, b"before");
    f.recorder.configure(Settings::default()).unwrap();
    c.append(Side::Response, b"off");
    f.enable(false, false);
    c.append(Side::Response, b"after");
    let record = f.detail(&c);
    assert_eq!(record.state, "incomplete");
    assert_eq!(record.capture_error.as_deref(), Some("stopped"));
    assert_eq!(
        f.snapshot(&c).body_page(Side::Response, 0).unwrap().text,
        "before"
    );
}
#[test]
fn promotion_coordinates_active_writes_and_retention_is_snapshotted() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    c.append(Side::Response, b"first");
    f.enable(true, false);
    assert!(!f.detail(&c).retained);
    let id = f.detail(&c).id;
    f.recorder.retain(id.clone()).unwrap();
    c.append(Side::Response, b"second");
    c.finish("complete");
    assert!(f.detail(&c).retained);
    assert!(!f.root.path().join("recent").join(&id).exists());
    assert_eq!(
        f.snapshot(&c).body_page(Side::Response, 0).unwrap().text,
        "firstsecond"
    );
    let next = f.begin();
    next.finish("complete");
    assert!(f.detail(&next).retained);
}
#[test]
fn completed_recent_records_rotate_but_retained_and_active_records_survive() {
    let f = Fixture::new();
    f.enable(false, false);
    f.recorder
        .configure(Settings {
            enabled: true,
            recent_count: 100,
            ..Settings::default()
        })
        .unwrap();
    let kept = f.begin();
    kept.finish("complete");
    let kept_id = f.detail(&kept).id;
    f.recorder.retain(kept_id.clone()).unwrap();
    let active = f.begin();
    let active_id = f.detail(&active).id;
    let mut first = String::new();
    for index in 0..101 {
        let c = f.begin();
        c.finish("complete");
        let id = f.detail(&c).id;
        if index == 0 {
            first = id;
        }
    }
    assert!(f.recorder.record(first).is_err());
    assert!(f.recorder.record(active_id).is_ok());
    assert!(f.recorder.record(kept_id).unwrap().retained);
}
#[test]
fn binary_preview_and_text_body_limits_are_independent() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    let mut headers = http::HeaderMap::new();
    headers.insert("content-type", "application/octet-stream".parse().unwrap());
    c.response(200, "https://synthetic.example", &headers);
    c.append(Side::Response, &vec![0xff; 100_000]);
    c.append(Side::Request, b"still-recorded");
    c.finish("complete");
    let record = f.detail(&c);
    assert_eq!(record.response.body.captured_bytes, BINARY_BYTES);
    assert_eq!(record.response.body.observed_bytes, 100_000);
    assert!(record.response.body.truncated);
    assert_eq!(
        f.snapshot(&c).body_page(Side::Request, 0).unwrap().text,
        "still-recorded"
    );
    let mut output = Vec::new();
    f.snapshot(&c).export_json(&mut output).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["response"]["body"]["encoding"], "hex");
}
#[test]
fn queue_overflow_is_bounded_and_has_a_terminal_reason_without_backpressure() {
    let f = Fixture::new();
    f.enable(false, false);
    let (release, wait) = mpsc::channel();
    let (ready, started) = mpsc::channel();
    f.recorder
        .0
        .sender
        .send(Event::Job(Box::new(move |_| {
            ready.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        })))
        .unwrap();
    started.recv().unwrap();
    let c = f.begin();
    let began = Instant::now();
    for _ in 0..8192 {
        c.append(Side::Response, &[b'x'; 16 * 1024]);
    }
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(f.recorder.0.queued.load(Ordering::Relaxed) <= QUEUE_BYTES);
    c.finish("complete");
    release.send(()).unwrap();
    let record = f.detail(&c);
    assert_eq!(record.state, "complete");
    assert_eq!(record.capture_error.as_deref(), Some("queue-full"));
    assert!(record.response.body.truncated);
    assert_eq!(record.response.body.observed_bytes, 128 * MIB);
}
#[test]
fn disk_failure_does_not_escape_capture_or_hide_the_incomplete_reason() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    let id = f.detail(&c).id;
    let path = f.root.path().join("recent").join(id).join("response");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    c.append(Side::Response, b"not-written");
    c.finish("complete");
    let record = f.detail(&c);
    assert_eq!(record.state, "complete");
    assert_eq!(record.capture_error.as_deref(), Some("write-failed"));
    assert!(record.response.body.truncated);
}
#[test]
fn pages_do_not_split_utf8_and_export_escapes_across_64k_boundaries() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f.begin();
    let text = "x".repeat(PAGE_BYTES - 1) + "😀tail\"\\\n";
    for chunk in text.as_bytes().chunks(16384) {
        c.append(Side::Response, chunk);
    }
    c.finish("complete");
    let mut snapshot = f.snapshot(&c);
    let first = snapshot.body_page(Side::Response, 0).unwrap();
    assert_eq!(first.next_offset, (PAGE_BYTES - 1) as u64);
    let next = snapshot
        .body_page(Side::Response, first.next_offset)
        .unwrap();
    assert_eq!(first.text + &next.text, text);
    let mut output = Vec::new();
    f.snapshot(&c).export_json(&mut output).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["response"]["body"]["data"], text);
}
#[test]
fn restart_cleans_recent_records_and_keeps_retained_records() {
    let f = Fixture::new();
    f.enable(false, false);
    let temp = f.begin();
    temp.finish("complete");
    let temp_id = f.detail(&temp).id;
    f.enable(true, false);
    let kept = f.begin();
    kept.append(Side::Response, b"kept");
    kept.finish("complete");
    let id = f.detail(&kept).id;
    f.recorder.shutdown();
    let reopened =
        Recorder::open(f.root.path().join("recent"), f.root.path().join("kept")).unwrap();
    assert!(reopened.record(temp_id).is_err());
    assert!(reopened.record(id).unwrap().retained);
    reopened.shutdown();
}
#[test]
fn empty_websocket_control_messages_are_observable_and_event_pages_are_bounded() {
    let f = Fixture::new();
    f.enable(false, false);
    let c = f
        .recorder
        .begin(
            "ws",
            "WS",
            "wss://synthetic.example",
            &http::HeaderMap::new(),
        )
        .guard();
    for _ in 0..70 {
        c.append_message(Side::Response, b"", Some("close"));
    }
    c.finish("complete");
    let mut snapshot = f.snapshot(&c);
    let first = snapshot.observations(0).unwrap();
    assert_eq!(first.text.lines().count(), 50);
    let second = snapshot.observations(first.next_offset).unwrap();
    assert_eq!(second.text.lines().count(), 20);
}

#[test]
fn gzip_decodes_in_the_writer_and_a_broken_tail_is_explicit() {
    use std::io::Write;
    let f = Fixture::new();
    f.enable(false, true);
    let raw = "data: {\"delta\":\"가나다\"}\n\n".repeat(10000);
    let mut zip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    zip.write_all(raw.as_bytes()).unwrap();
    let zip = zip.finish().unwrap();
    for broken in [false, true] {
        let c = f.begin();
        let mut headers = text_headers();
        headers.insert("content-encoding", "gzip".parse().unwrap());
        c.response(200, "https://synthetic.example", &headers);
        let source = if broken {
            &zip[..zip.len() - 8]
        } else {
            zip.as_slice()
        };
        for chunk in source.chunks(7) {
            c.append(Side::Response, chunk);
        }
        c.finish("complete");
        let record = f.detail(&c);
        assert_eq!(record.response.body.representation, "decoded-gzip");
        assert_eq!(record.response.body.observed_bytes, raw.len() as u64);
        assert_eq!(f.snapshot(&c).copy_body(Side::Response).unwrap(), raw);
        assert_eq!(
            record.capture_error.as_deref(),
            if broken { Some("decode-failed") } else { None }
        );
    }
}
#[test]
fn websocket_binary_export_is_lossless_and_text_messages_remain_readable() {
    let f = Fixture::new();
    f.enable(false, true);
    let c = f
        .recorder
        .begin(
            "ws",
            "WS",
            "wss://synthetic.example",
            &http::HeaderMap::new(),
        )
        .guard();
    c.append_message(Side::Response, b"hello", Some("text"));
    c.append_message(Side::Response, &[0, 255, 128], Some("binary"));
    c.finish("complete");
    let page = f
        .snapshot(&c)
        .body_range(Side::Response, 0, 5, Some(false))
        .unwrap();
    assert_eq!(page.text, "hello");
    let mut json = Vec::new();
    f.snapshot(&c).export_json(&mut json).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(json["response"]["body"]["encoding"], "hex");
    assert_eq!(json["response"]["body"]["data"], "68656c6c6f00ff80");
}
#[test]
fn readers_pin_files_and_clipboard_rejects_over_16_mib_without_loading_them() {
    let f = Fixture::new();
    f.enable(false, true);
    let c = f.begin();
    c.finish("complete");
    let id = f.detail(&c).id;
    let a = f.snapshot(&c);
    let b = f.snapshot(&c);
    let d = f.snapshot(&c);
    let e = f.snapshot(&c);
    assert!(f.recorder.snapshot(id.clone()).is_err());
    assert_eq!(
        f.recorder.delete(id.clone()),
        Err("network-log-readers-busy".into())
    );
    drop((a, b, d, e));
    let task_id = id.clone();
    f.recorder
        .job(move |store| {
            let mut record = store.record(&task_id)?;
            record.response.body.captured_bytes = 16 * MIB + 1;
            std::fs::OpenOptions::new()
                .write(true)
                .open(store.dir(&record).join("response"))
                .unwrap()
                .set_len(16 * MIB + 1)
                .unwrap();
            store.update(&record)
        })
        .unwrap();
    assert_eq!(
        f.snapshot(&c).copy_body(Side::Response),
        Err("network-log-clipboard-too-large".into())
    );
    f.recorder.delete(id).unwrap();
}
#[test]
fn quota_stops_capture_and_preserves_existing_retained_requests() {
    let f = Fixture::new();
    f.enable(true, true);
    let kept = f.begin();
    kept.append(Side::Response, b"preserved");
    kept.finish("complete");
    let id = f.detail(&kept).id;
    f.recorder
        .configure(Settings {
            enabled: true,
            retain: true,
            retained_mib: 128,
            ..Settings::default()
        })
        .unwrap();
    let c = f.begin();
    let task_id = f.detail(&c).id;
    f.recorder
        .job(move |store| {
            let mut record = store.record(&task_id)?;
            // A sparse synthetic file reaches the budget without allocating a giant fixture.
            let length = 128 * MIB - 3 * MIB;
            std::fs::OpenOptions::new()
                .write(true)
                .open(store.dir(&record).join("response"))
                .unwrap()
                .set_len(length)
                .unwrap();
            record.response.body.captured_bytes = length;
            store.bytes[1] += length;
            store.update(&record)
        })
        .unwrap();
    for _ in 0..32 {
        c.append(Side::Response, &[0; 65536]);
        let _ = f.detail(&c);
    }
    c.finish("complete");
    assert_eq!(f.detail(&c).capture_error.as_deref(), Some("quota"));
    assert_eq!(
        f.recorder
            .snapshot(id)
            .unwrap()
            .copy_body(Side::Response)
            .unwrap(),
        "preserved"
    );
    assert!(f.recorder.list(Query::default()).unwrap().retained_bytes <= 128 * MIB);
}

#[cfg(feature = "reqwest")]
#[tokio::test(flavor = "current_thread")]
async fn real_http_keeps_request_body_redirect_url_decoding_and_cancellation() {
    use crate::reqwest::RequestBuilderExt;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let f = Fixture::new();
    f.enable(false, true);
    crate::install(f.recorder.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let redirect = format!("{base}/final");
    let expected_url = redirect.clone();
    let server = tokio::spawn(async move {
        for index in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let size = socket.read(&mut buffer).await.unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let body_length = if index < 2 { 17 } else { 0 };
                    if request.len() >= end + 4 + body_length {
                        break;
                    }
                }
            }
            if index < 2 {
                assert!(request.ends_with(b"synthetic-payload"));
            }
            match index {
                0 => socket.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap(),
                1 | 2 => {
                    let mut zip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()); zip.write_all(b"data: readable\n\n").unwrap(); let zip = zip.finish().unwrap();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", zip.len()).as_bytes()).await.unwrap(); socket.write_all(&zip).await.unwrap();
                },
                _ => { socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\ndata\r\n").await.unwrap(); tokio::time::sleep(Duration::from_millis(100)).await; }
            }
        }
    });
    let client = ::reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(format!("{base}/redirect"))
        .body("synthetic-payload")
        .send_logged("adapter-auto-gzip")
        .await
        .unwrap();
    assert_eq!(response.url().as_str(), expected_url);
    assert_eq!(response.text().await.unwrap(), "data: readable\n\n");
    let client = ::reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .build()
        .unwrap();
    let response = client
        .get(format!("{base}/compressed"))
        .send_logged("adapter-raw-gzip")
        .await
        .unwrap();
    assert_eq!(response.headers()["content-encoding"], "gzip");
    assert_eq!(&response.bytes().await.unwrap()[..2], &[31, 139]);
    let mut response = client
        .get(format!("{base}/cancel"))
        .send_logged("adapter-cancel")
        .await
        .unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap(), "data");
    drop(response);
    server.await.unwrap();
    let rows = f.recorder.list(Query::default()).unwrap();
    assert_eq!(rows.items.len(), 3);
    for row in rows.items {
        let record = f.recorder.record(row.id.clone()).unwrap();
        let text = f
            .recorder
            .snapshot(row.id)
            .unwrap()
            .copy_body(Side::Response)
            .unwrap();
        if record.source == "adapter-cancel" {
            assert_eq!(record.state, "incomplete");
            assert_eq!(text, "data");
        } else {
            assert_eq!(record.state, "complete");
            assert_eq!(text, "data: readable\n\n");
        }
    }
}

#[test]
fn large_export_streams_512_mib_without_a_full_body_allocation() {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len() as u64;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let f = Fixture::new();
    f.enable(false, true);
    let c = f.begin();
    c.finish("complete");
    let id = f.detail(&c).id;
    f.recorder
        .job(move |store| {
            let mut record = store.record(&id)?;
            record.response.body.captured_bytes = 512 * MIB;
            record.response.body.observed_bytes = 512 * MIB;
            std::fs::OpenOptions::new()
                .write(true)
                .open(store.dir(&record).join("response"))
                .unwrap()
                .set_len(512 * MIB)
                .unwrap();
            store.bytes[0] += 512 * MIB;
            store.update(&record)
        })
        .unwrap();
    let mut output = Counter(0);
    f.snapshot(&c).export_json(&mut output).unwrap();
    // NUL is emitted as six JSON escape bytes, without retaining the output.
    assert!(output.0 >= 512 * MIB * 6 && output.0 < 512 * MIB * 6 + MIB);
}
#[test]
fn concurrent_streams_and_a_single_enormous_chunk_keep_the_shared_queue_bounded() {
    let f = Fixture::new();
    f.enable(false, true);
    let (release, wait) = mpsc::channel();
    let (ready, started) = mpsc::channel();
    f.recorder
        .0
        .sender
        .send(Event::Job(Box::new(move |_| {
            ready.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
        })))
        .unwrap();
    started.recv().unwrap();
    let captures: Vec<_> = (0..4).map(|_| f.begin()).collect();
    let huge = vec![b'x'; 64 * 1024 * 1024];
    std::thread::scope(|scope| {
        for capture in &captures {
            let bytes = &huge;
            scope.spawn(move || {
                capture.append(Side::Response, bytes);
                capture.finish("complete");
            });
        }
    });
    assert!(f.recorder.0.queued.load(Ordering::Relaxed) <= QUEUE_BYTES);
    drop(huge);
    release.send(()).unwrap();
    for capture in captures {
        let record = f.detail(&capture);
        assert_eq!(record.response.body.observed_bytes, 64 * MIB);
        assert_eq!(record.capture_error.as_deref(), Some("queue-full"));
        assert!(record.response.body.truncated);
    }
}

#[test]
fn interrupted_retained_record_recovers_only_committed_bytes_and_cleans_cache() {
    let f = Fixture::new();
    f.enable(true, true);
    let c = f.begin();
    c.append(Side::Response, b"kept");
    let record = f.detail(&c);
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("recent");
    let kept = root.path().join("retained");
    {
        let mut store = storage::Store::open(cache.clone(), kept.clone()).unwrap();
        store.insert(&record, &Settings::default()).unwrap();
        fs::write(
            store.dir(&record).join("response"),
            b"kept-uncommitted-tail",
        )
        .unwrap();
        fs::write(cache.join("unflushed"), b"temporary").unwrap();
    }
    let recovered = Recorder::open(cache.clone(), kept.clone()).unwrap();
    let saved = recovered.record(record.id.clone()).unwrap();
    assert_eq!(saved.state, "incomplete");
    assert_eq!(saved.capture_error.as_deref(), Some("interrupted"));
    assert_eq!(
        recovered
            .snapshot(record.id.clone())
            .unwrap()
            .copy_body(Side::Response)
            .unwrap(),
        "kept"
    );
    assert_eq!(
        fs::metadata(kept.join(record.id).join("response"))
            .unwrap()
            .len(),
        4
    );
    assert!(!cache.join("unflushed").exists());
    recovered.shutdown();
}
