use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HashDomain {
    pub calls: u64,
    pub bytes: u64,
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
        assert!(sample.validate_routine_invariants().is_ok());
        sample.incomplete.push("native/stream".into());
        assert!(sample.hash_totals().is_err());
    }
    #[test]
    fn growth_requires_precise_control_attribution_and_never_more_calls() {
        let mut a = NativeObservation { asset_body_opens:Some(0), asset_body_bytes_read:Some(0),
            shared_head_reads:Some(0), ..Default::default() };
        a.insert_domain("C", "intent", 1, 100);
        let mut b = a.clone();
        b.hashes.get_mut("C/intent").unwrap().bytes += 10;
        assert!(assert_frozen_growth(&a, &b, &[]).is_err());
        assert!(assert_frozen_growth(&a, &b, &["C/intent"]).is_ok());
        b.hashes.get_mut("C/intent").unwrap().calls += 1;
        assert!(assert_frozen_growth(&a, &b, &["C/intent"]).is_err());
    }
}
