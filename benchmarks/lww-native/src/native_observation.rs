use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HashDomain {
    pub calls: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyDomain {
    pub open_attempts: u64,
    pub opens: u64,
    pub failed_opens: u64,
    pub unknown_open_results: u64,
    pub read_operations: u64,
    pub read_bytes: u64,
    pub incomplete_reads: u64,
    pub escaped_handles: u64,
    pub escaped_paths: u64,
    pub outstanding_readers: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectBodyDomain {
    pub purposes: Vec<String>,
    pub work: BodyDomain,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NativeObservation {
    pub units_visited: u64,
    pub hashes: BTreeMap<String, HashDomain>,
    pub incomplete: Vec<String>,
    pub requests: u64,
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub messages_read: u64,
    pub message_bytes_read: u64,
    pub pages_written: u64,
    pub pages_reused: u64,
    pub asset_body_opens: Option<u64>,
    pub asset_body_bytes_read: Option<u64>,
    pub shared_head_reads: Option<u64>,
    pub body_scope_complete: Option<bool>,
    pub body_scope_thread: Option<String>,
    pub body_domains: BTreeMap<String, BodyDomain>,
    pub body_objects: BTreeMap<String, ObjectBodyDomain>,
    pub body_asset_work: BodyDomain,
    pub body_control_work: BodyDomain,
    pub body_owned_work: BodyDomain,
    pub body_unknown_work: BodyDomain,
    pub body_unattributed_managed: BodyDomain,
    pub body_scope_violations: u64,
    pub body_stat_requests: u64,
    pub body_batch_stat_requests: u64,
    pub body_presence_queries: u64,
}

impl NativeObservation {
    pub fn insert_domain(&mut self, owner: &str, domain: &str, calls: u64, bytes: u64) {
        let key = format!("{owner}/{domain}");
        assert!(self.hashes.insert(key, HashDomain { calls, bytes }).is_none(), "duplicate hash domain");
    }

    pub fn hash_totals(&self) -> Result<HashDomain, String> {
        if !self.incomplete.is_empty() {
            return Err(format!("incomplete SHA inputs: {:?}", self.incomplete));
        }
        self.hashes.values().try_fold(HashDomain::default(), |mut total, domain| {
            total.calls = total.calls.checked_add(domain.calls).ok_or("hash call overflow")?;
            total.bytes = total.bytes.checked_add(domain.bytes).ok_or("hash byte overflow")?;
            Ok(total)
        })
    }

    pub fn validate_routine_invariants(&self) -> Result<(), String> {
        self.hash_totals()?;
        if self.body_scope_complete != Some(true) || self.body_scope_violations != 0
            || self.body_unknown_work != BodyDomain::default()
            || self.body_unattributed_managed != BodyDomain::default()
            || self.body_owned_work != BodyDomain::default()
            || self.body_domains.get("owned").is_some_and(|work| work != &BodyDomain::default())
            || self.body_objects.values().any(|object| object.purposes.is_empty()
                && object.work != BodyDomain::default())
            || self.body_domains.values().chain(self.body_objects.values().map(|object| &object.work)).any(|domain|
            domain.unknown_open_results != 0 || domain.incomplete_reads != 0
                || domain.escaped_handles != 0 || domain.escaped_paths != 0
                || domain.outstanding_readers != 0) {
            return Err("incomplete or unobserved CAS body scope".into());
        }
        if self.asset_body_opens != Some(self.body_asset_work.open_attempts)
            || self.asset_body_bytes_read != Some(self.body_asset_work.read_bytes) {
            return Err("inconsistent asset-purpose body totals".into());
        }
        for (domain, work) in &self.hashes {
            if (domain.contains("binding") || domain.contains("catalog_proof") || domain.contains("generation_digest"))
                && (work.calls != 0 || work.bytes != 0)
            {
                return Err(format!("routine binding proof: {domain}"));
            }
        }
        for (name, value) in [("asset body opens", self.asset_body_opens),
            ("asset body bytes", self.asset_body_bytes_read), ("shared head reads", self.shared_head_reads)] {
            match value {
                Some(0) => {},
                Some(_) => return Err(format!("nonzero {name}")),
                None => return Err(format!("unobserved {name}")),
            }
        }
        Ok(())
    }
}

pub fn assert_frozen_growth(a: &NativeObservation, b: &NativeObservation,
    allowed_control_domains: &[&str]) -> Result<(), String> {
    a.validate_routine_invariants()?;
    b.validate_routine_invariants()?;
    if b.units_visited > a.units_visited || b.requests > a.requests {
        return Err("unit/request growth".into());
    }
    let mut control_growth = 0u64;
    for (key, larger) in &b.hashes {
        let smaller = a.hashes.get(key).cloned().unwrap_or_default();
        if larger.calls > smaller.calls { return Err(format!("hash call growth: {key}")); }
        if larger.bytes > smaller.bytes {
            if !allowed_control_domains.contains(&key.as_str()) {
                return Err(format!("unattributed hash byte growth: {key}"));
            }
            control_growth = control_growth.checked_add(larger.bytes - smaller.bytes).ok_or("growth overflow")?;
        }
    }
    if control_growth > 8192 { return Err("control metadata growth exceeds 8 KiB".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_and_unobserved_inputs_cannot_pass() {
        let mut sample = NativeObservation::default();
        assert!(sample.validate_routine_invariants().is_err());
        sample.asset_body_opens = Some(0);
        sample.asset_body_bytes_read = Some(0);
        sample.shared_head_reads = Some(0);
        sample.body_scope_complete = Some(true);
        assert!(sample.validate_routine_invariants().is_ok());
        sample.incomplete.push("native/stream".into());
        assert!(sample.hash_totals().is_err());
    }
    #[test]
    fn role_coverage_and_reader_lifecycle_remain_fail_closed() {
        let mut sample = NativeObservation {asset_body_opens:Some(0), asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0), body_scope_complete:Some(true), ..Default::default()};
        sample.body_control_work = BodyDomain {open_attempts:1, opens:1, read_operations:1,
            read_bytes:22, ..Default::default()};
        sample.body_domains.insert("managed".into(), sample.body_control_work.clone());
        sample.validate_routine_invariants().unwrap();
        sample.body_unknown_work.open_attempts = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_unknown_work = BodyDomain::default();
        sample.body_domains.get_mut("managed").unwrap().outstanding_readers = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_domains.get_mut("managed").unwrap().outstanding_readers = 0;
        sample.body_scope_violations = 1;
        assert!(sample.validate_routine_invariants().is_err());
        sample.body_scope_violations = 0;
        sample.body_owned_work.read_bytes = 1;
        assert!(sample.validate_routine_invariants().is_err());
    }

    #[test]
    fn growth_requires_precise_control_attribution_and_never_more_calls() {
        let mut a = NativeObservation { asset_body_opens:Some(0), asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0), body_scope_complete:Some(true), ..Default::default() };
        a.insert_domain("C", "intent", 1, 100);
        let mut b = a.clone();
        b.hashes.get_mut("C/intent").unwrap().bytes += 10;
        assert!(assert_frozen_growth(&a, &b, &[]).is_err());
        assert!(assert_frozen_growth(&a, &b, &["C/intent"]).is_ok());
        b.hashes.get_mut("C/intent").unwrap().calls += 1;
        assert!(assert_frozen_growth(&a, &b, &["C/intent"]).is_err());
    }
}
