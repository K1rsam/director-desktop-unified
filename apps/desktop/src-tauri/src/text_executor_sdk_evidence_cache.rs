//! Bounded memoization of pure SDK evidence parsing/compilation. The caller
//! still reads every file and checks DB, receipt, event and CAS evidence.
//! A non-Send guard confines these values to one synchronous validation stack.
use super::CompiledCodexInput;
use crate::text_transport::FrozenAttempt;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
#[derive(Default)]
struct Cache {
    depth: usize,
    bytes: usize,
    entries: usize,
    attempts: HashMap<PathBuf, HashMap<Vec<u8>, FrozenAttempt>>,
    compiled: HashMap<Vec<u8>, (String, String)>,
    pure_domains: HashSet<[u8; 32]>,
}
thread_local! { static CURRENT: RefCell<Option<Cache>> = const { RefCell::new(None) }; }

pub(crate) struct Scope {
    _not_send: PhantomData<Rc<()>>,
}
pub(crate) fn scope() -> Scope {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        current.get_or_insert_with(Cache::default).depth += 1;
    });
    Scope {
        _not_send: PhantomData,
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            let cache = current.as_mut().expect("synchronous evidence scope");
            cache.depth -= 1;
            if cache.depth == 0 {
                *current = None;
            }
        });
    }
}
pub(crate) fn active() -> bool {
    CURRENT.with(|current| current.borrow().is_some())
}
pub(crate) fn attempt(path: &Path, bytes: &[u8]) -> Option<FrozenAttempt> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()?
            .attempts
            .get(path)?
            .get(bytes)
            .cloned()
    })
}
pub(crate) fn remember_attempt(path: &Path, bytes: Vec<u8>, value: &FrozenAttempt) {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let Some(cache) = current.as_mut() else {
            return;
        };
        // Count the serialized envelope plus a conservative allowance for its
        // parsed representation; admission failure merely leaves caching off.
        let size = bytes.len().saturating_mul(4);
        if cache.entries >= MAX_ENTRIES || cache.bytes.saturating_add(size) > MAX_BYTES {
            return;
        }
        let values = cache.attempts.entry(path.to_path_buf()).or_default();
        if values.contains_key(&bytes) {
            return;
        }
        values.insert(bytes, value.clone());
        cache.entries += 1;
        cache.bytes += size;
    });
}
pub(crate) fn compiled(bytes: &[u8]) -> Option<CompiledCodexInput> {
    CURRENT.with(|current| {
        let current = current.borrow();
        let (prompt, schema) = current.as_ref()?.compiled.get(bytes)?;
        Some(CompiledCodexInput {
            prompt: prompt.clone(),
            schema: schema.clone(),
        })
    })
}
pub(crate) fn remember_compiled(bytes: Vec<u8>, value: &CompiledCodexInput) {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let Some(cache) = current.as_mut() else {
            return;
        };
        let size = bytes
            .len()
            .saturating_add(value.prompt.len())
            .saturating_add(value.schema.len());
        if cache.entries >= MAX_ENTRIES
            || cache.bytes.saturating_add(size) > MAX_BYTES
            || cache.compiled.contains_key(&bytes)
        {
            return;
        }
        cache
            .compiled
            .insert(bytes, (value.prompt.clone(), value.schema.clone()));
        cache.entries += 1;
        cache.bytes += size;
    });
}

/// Memoize only a pure domain validator in this synchronous call stack. Callers
/// serialize a namespaced tuple of *all* validator inputs; no DB/worker-file,
/// authorization ledger, checkpoint, UNKNOWN or CAS check belongs here.
/// Fresh evidence is still read on every SDK entry, even when this succeeds.
pub(crate) fn pure_domain(
    full_inputs: &[u8],
    validate: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let key: [u8; 32] = Sha256::digest(full_inputs).into();
    let verified = CURRENT.with(|current| {
        current.borrow().as_ref().is_some_and(|cache| cache.pure_domains.contains(&key))
    });
    if verified { return Ok(()); }
    validate()?;
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let Some(cache) = current.as_mut() else { return; };
        if cache.entries < MAX_ENTRIES && cache.bytes.saturating_add(32) <= MAX_BYTES
            && cache.pure_domains.insert(key)
        {
            cache.entries += 1;
            cache.bytes += 32;
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pipeline::TextRequest, text_executor::compile_sdk_attempt_input, text_transport::Backend,
    };
    use serde_json::json;
    fn frozen() -> FrozenAttempt {
        FrozenAttempt::prepare("synthetic-attempt", "synthetic-workflow", "synthetic-node", Backend::CodexSdk,
            TextRequest { stage:"synthetic".into(), skill:"synthetic".into(), model:"offline".into(), reasoning:"medium".into(),
                input:json!({"story":"unchanged synthetic"}), schema:json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),rule_snapshot:json!({"rules":[]}) }, &json!({"provider":"synthetic-never-dispatch"})).unwrap()
    }
    fn count() -> usize {
        CURRENT.with(|current| current.borrow().as_ref().map_or(0, |c| c.entries))
    }
    #[test]
    fn sdk_evidence_cache_exact_bytes_detect_file_drift_and_drop_at_scope_end() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("attempt.json");
        let value = frozen();
        value.persist(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(!active());
        {
            let _scope = scope();
            let first = FrozenAttempt::load(&path).unwrap();
            assert_eq!(count(), 1);
            {
                let _nested = scope();
                assert_eq!(
                    FrozenAttempt::load(&path).unwrap().attempt_envelope_hash,
                    first.attempt_envelope_hash
                );
                assert_eq!(count(), 1);
            }
            std::fs::write(&path, "{}").unwrap();
            assert!(FrozenAttempt::load(&path).is_err());
            assert_eq!(count(), 1);
            std::fs::write(&path, &original).unwrap();
            assert!(FrozenAttempt::load(&path).is_ok());
            let other = temp.path().join("other.json");
            std::fs::write(&other, &original).unwrap();
            assert!(FrozenAttempt::load(&other).is_ok());
            assert_eq!(count(), 2);
        }
        assert!(!active());
        assert_eq!(count(), 0);
        std::fs::write(&path, "{}").unwrap();
        let _new = scope();
        assert!(FrozenAttempt::load(&path).is_err());
        assert_eq!(count(), 0);
    }
    #[test]
    fn sdk_evidence_cache_compiler_keys_all_frozen_fields_and_never_caches_failure() {
        let value = frozen();
        let uncached = compile_sdk_attempt_input(&value).unwrap();
        assert_eq!(count(), 0);
        {
            let _scope = scope();
            let first = compile_sdk_attempt_input(&value).unwrap();
            assert_eq!(count(), 1);
            let second = compile_sdk_attempt_input(&value).unwrap();
            assert_eq!((first.prompt, first.schema), (second.prompt, second.schema));
            for fault in ["input", "schema", "identity", "state", "format"] {
                let mut changed = value.clone();
                match fault {
                    "input" => changed.request.input["story"] = json!("drift"),
                    "schema" => changed.request.schema["additionalProperties"] = json!(true),
                    "identity" => changed.runtime_identity["provider"] = json!("drift"),
                    "state" => changed.state = crate::text_transport::AttemptState::Completed,
                    "format" => changed.compiled_input_format = Some("unknown".into()),
                    _ => unreachable!(),
                }
                assert!(compile_sdk_attempt_input(&changed).is_err(), "{fault}");
                assert_eq!(count(), 1);
            }
            let repeated = compile_sdk_attempt_input(&value).unwrap();
            assert_eq!(
                (repeated.prompt, repeated.schema),
                (uncached.prompt, uncached.schema)
            );
        }
        assert_eq!(count(), 0);
    }
    #[test]
    fn sdk_evidence_cache_is_bounded_and_threads_do_not_share_a_scope() {
        let _scope = scope();
        let value = frozen();
        remember_attempt(Path::new("large"), vec![0; MAX_BYTES], &value);
        assert_eq!(count(), 0);
        let compiled = CompiledCodexInput {
            prompt: "p".into(),
            schema: "s".into(),
        };
        for n in 0..MAX_ENTRIES + 5 {
            remember_compiled(n.to_le_bytes().to_vec(), &compiled);
        }
        assert_eq!(count(), MAX_ENTRIES);
        std::thread::spawn(|| {
            assert!(!active());
            assert_eq!(count(), 0);
            let _scope = scope();
            assert_eq!(count(), 0);
        })
        .join()
        .unwrap();
        assert_eq!(count(), MAX_ENTRIES);
    }

    #[test]
    fn sdk_evidence_pure_domain_rechecks_drift_failure_and_each_new_scope() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let validate = |valid: bool| {
            calls.set(calls.get() + 1);
            if valid { Ok(()) } else { Err("changed pure input".to_owned()) }
        };
        let a = br#"["correction-context",{"raw":"original","rules":"v1"}]"#;
        let b = br#"["correction-context",{"raw":"changed","rules":"v1"}]"#;
        let other = br#"["correction-transition",{"raw":"original","rules":"v1"}]"#;
        {
            let _scope = scope();
            pure_domain(a, || validate(true)).unwrap();
            {
                let _nested = scope();
                pure_domain(a, || validate(true)).unwrap();
                assert_eq!(calls.get(), 1);
                assert!(pure_domain(b, || validate(false)).is_err());
                assert!(pure_domain(b, || validate(false)).is_err());
                pure_domain(other, || validate(true)).unwrap();
                assert_eq!(calls.get(), 4);
            }
        }
        {
            let _fresh = scope();
            pure_domain(a, || validate(true)).unwrap();
            assert_eq!(calls.get(), 5);
        }
        pure_domain(a, || validate(true)).unwrap();
        pure_domain(a, || validate(true)).unwrap();
        assert_eq!(calls.get(), 7, "outside a stack no result persists");
    }
}
