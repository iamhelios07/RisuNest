use risunest_sync_wire::{
    canonical, hash,
    lww::*,
    stamp::Stamp,
    unit::{UnitKey, UnitValue},
};
fn request() -> PushRequest {
    let writer = "00000000-0000-4000-8000-000000000001";
    PushRequest {
        library_id: "library".into(),
        writer_id: writer.into(),
        operation_id: "operation".into(),
        changes: vec![UnitChange {
            key: UnitKey::new(&["root", "synthetic"]).unwrap(),
            stamp: Stamp {
                physical_ms: 9_007_199_254_740_993.into(),
                logical: 3,
                writer_id: writer.into(),
            },
            value: UnitValue::inline(br#"{"number":1}"#).unwrap(),
        }],
    }
}
#[test]
fn push_control_round_trip_uses_decimal_strings_and_base64_payload() {
    let request = request();
    request.validate().unwrap();
    let encoded = canonical::encode(&request).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        json["changes"][0]["stamp"]["physicalMs"],
        "9007199254740993"
    );
    assert_eq!(json["changes"][0]["stamp"]["logical"], "3");
    assert_eq!(
        canonical::decode::<PushRequest>(&encoded, 4096).unwrap(),
        request
    );
    assert_eq!(request.digest().unwrap(), hash(&encoded));
}
#[test]
fn altered_operation_body_changes_digest_and_duplicate_units_or_writers_fail() {
    let request = request();
    let mut altered = request.clone();
    altered.changes[0].value = UnitValue::Deleted;
    assert_ne!(request.digest().unwrap(), altered.digest().unwrap());
    altered.changes.push(altered.changes[0].clone());
    assert_eq!(altered.validate().unwrap_err().0, "duplicate-unit-key");
    let mut altered = request;
    altered.changes[0].stamp.writer_id = "00000000-0000-4000-8000-000000000002".into();
    assert_eq!(altered.validate().unwrap_err().0, "writer-mismatch");
}
#[test]
fn terminal_receipts_and_cancellation_round_trip_without_number_controls() {
    let request = request();
    let digest = request.digest().unwrap();
    let accepted = OperationReceipt::Accepted {
        body_digest: digest.clone(),
        receipt: PushReceipt {
            operation_id: request.operation_id.clone(),
            seq: 4.into(),
            accepted_keys: vec![request.changes[0].key.clone()],
            server_time_ms: 100.into(),
        },
    };
    let rejected = OperationReceipt::Rejected {
        operation_id: request.operation_id,
        body_digest: digest.clone(),
        error: "operation-cancelled".into(),
        server_time_ms: 100.into(),
    };
    for receipt in [accepted, rejected] {
        assert_eq!(receipt.body_digest(), digest);
        assert_eq!(
            canonical::decode::<OperationReceipt>(&canonical::encode(&receipt).unwrap(), 4096)
                .unwrap(),
            receipt
        );
    }
    let cancel = CancelOperationRequest {
        body_digest: digest,
    };
    cancel.validate().unwrap();
    assert!(CancelOperationRequest {
        body_digest: "bad".into()
    }
    .validate()
    .is_err());
}
#[test]
fn notifications_and_pages_keep_precise_cursors() {
    let notice = SeqNotification::Seq {
        seq: 9_007_199_254_740_993.into(),
    };
    assert_eq!(
        canonical::encode(&notice).unwrap(),
        br#"{"seq":"9007199254740993","type":"seq"}"#
    );
    let request = request();
    let unit = request.changes[0].clone();
    let page = StatePage {
        pin_id: "pin".into(),
        start_seq: 1.into(),
        items: vec![unit.clone()],
        next_key: Some(unit.key),
    };
    assert_eq!(
        canonical::decode::<StatePage>(&canonical::encode(&page).unwrap(), 4096).unwrap(),
        page
    );
    assert!(canonical::decode::<AckRequest>(br#"{"seq":1}"#, 4096).is_err());
    assert!(canonical::decode::<AckRequest>(br#"{"seq":"01"}"#, 4096).is_err());
    assert!(canonical::decode::<AckRequest>(br#"{"seq":"1","extra":true}"#, 4096).is_err());
}

#[test]
fn push_decode_rejects_unknown_deleted_value_fields() {
    let mut json = serde_json::to_value(request()).unwrap();
    json["changes"][0]["value"] = serde_json::json!({"kind":"deleted","bytes":"ignored"});
    let encoded = serde_json::to_vec(&json).unwrap();
    assert!(canonical::decode::<PushRequest>(&encoded, 4096).is_err());
}
