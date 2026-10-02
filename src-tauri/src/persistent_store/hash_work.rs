//! Same-thread SHA input accounting for native verification entries only.
//!
//! Reset immediately before the operation and take on that same thread. Worker
//! threads must return their own sample. Incomplete delegated calls invalidate
//! a total, even when the domains still contain useful observed partial work.
//! The byte component of `lww::take_work_metrics` aliases native_unit_envelope.
#![cfg(test)]

use risunest_sync_wire::{canonical, descriptor::BuiltReferenceTree, unit::{LwwDecision, UnitValue}, WireError};
use std::{cell::RefCell, collections::BTreeMap};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DomainWork {
    pub calls: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HashWork {
    pub domains: BTreeMap<&'static str, DomainWork>,
    pub incomplete: BTreeMap<&'static str, u64>,
}

thread_local! { static WORK: RefCell<HashWork> = RefCell::new(HashWork::default()); }

pub(crate) fn reset_hash_work() {
    WORK.with(|work| *work.borrow_mut() = HashWork::default());
}

pub(crate) fn take_hash_work() -> HashWork {
    WORK.with(|work| std::mem::take(&mut *work.borrow_mut()))
}

pub(crate) fn begin(domain: &'static str) {
    WORK.with(|work| work.borrow_mut().domains.entry(domain).or_default().calls += 1);
}

pub(crate) fn update(domain: &'static str, bytes: usize) {
    WORK.with(|work| work.borrow_mut().domains.entry(domain).or_default().bytes += bytes as u64);
}

pub(crate) fn observe(domain: &'static str, bytes: usize) {
    begin(domain);
    update(domain, bytes);
}

pub(crate) fn incomplete(domain: &'static str) {
    WORK.with(|work| *work.borrow_mut().incomplete.entry(domain).or_default() += 1);
}

// RecordDescriptor::bytes validates and canonically encodes, without hashing.
// UnitValue::validate hashes only after both these checks succeed, even when
// the final descriptor hash comparison fails.
pub(crate) fn validation(value: &UnitValue) {
    if let UnitValue::Object { descriptor_hash, descriptor } = value {
        if risunest_sync_wire::validate_hash(descriptor_hash).is_ok() {
            if let Ok(bytes) = descriptor.bytes() {
                observe("native_wire_descriptor_validation", bytes.len());
            }
        }
    }
}

pub(crate) fn identity(value: &UnitValue, result: &Result<String, WireError>) {
    validation(value);
    if result.is_ok() {
        match canonical::encode(value) {
            Ok(bytes) => observe("native_wire_identity", bytes.len()),
            Err(_) => incomplete("native_wire_identity"),
        }
    } else {
        incomplete("native_wire_identity");
    }
}

pub(crate) fn comparison(local: &UnitValue, remote: &UnitValue, result: &Result<LwwDecision, WireError>) {
    // An integrity disagreement happens after both complete identity calls.
    if result.is_ok() || result.as_ref().is_err_and(|error| error.0 == "equal-stamp-integrity") {
        for value in [local, remote] {
            validation(value);
            match canonical::encode(value) {
                Ok(bytes) => observe("native_wire_identity", bytes.len()),
                Err(_) => incomplete("native_wire_identity"),
            }
        }
    } else {
        incomplete("native_wire_identity");
    }
}

pub(crate) fn descriptor_creation(result: &Result<UnitValue, WireError>) {
    if let Ok(UnitValue::Object { descriptor, .. }) = result {
        match descriptor.bytes() {
            Ok(bytes) => observe("native_descriptor_creation", bytes.len()),
            Err(_) => incomplete("native_descriptor_creation"),
        }
    } else if result.is_err() {
        incomplete("native_descriptor_creation");
    }
}

pub(crate) fn reference_creation(result: &Result<BuiltReferenceTree, WireError>) {
    match result {
        Ok((_, objects)) => {
            for (_, bytes) in objects {
                observe("native_reference_create", bytes.len());
            }
        }
        Err(_) => incomplete("native_reference_create"),
    }
}

pub(crate) fn encoded_object<E>(domain: &'static str, result: &Result<risunest_external_storage_format::logical_records::EncodedLogicalObject, E>) {
    match result {
        Ok(object) => observe(domain, object.bytes.len()),
        Err(_) => incomplete(domain),
    }
}

pub(crate) fn decoded<E, T>(domain: &'static str, bytes: &[u8], result: &Result<T, E>) {
    if result.is_ok() {
        // Both decoders re-encode once and require exact canonical equality.
        observe(domain, bytes.len());
    } else {
        incomplete(domain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thread_local_samples_reset_and_take_independently() {
        reset_hash_work(); observe("outer", 3);
        let inner = std::thread::spawn(|| {
            assert_eq!(take_hash_work(), HashWork::default());
            observe("inner", 7); take_hash_work()
        }).join().unwrap();
        assert_eq!(inner.domains["inner"], DomainWork { calls: 1, bytes: 7 });
        let outer = take_hash_work();
        assert_eq!(outer.domains["outer"], DomainWork { calls: 1, bytes: 3 });
        assert!(!outer.domains.contains_key("inner"));
        assert_eq!(take_hash_work(), HashWork::default());
        observe("discarded", 1); reset_hash_work();
        assert_eq!(take_hash_work(), HashWork::default());
    }
}
