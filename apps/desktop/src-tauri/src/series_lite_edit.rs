//! Pure assembly port of series-storyboard-director-lite 0.1.2.
//!
//! Upstream: 6dea8d6c57dde38f04a4d71f05f4c039004dd4a9.
//! No model, filesystem, project adoption, or content-quality side effects.
//! Decimal spelling follows context_packet.py rather than an f64 round trip.
//! Native parser bounds nesting to 128 and decimal exponents to signed i64;
//! these explicit resource guards do not round or silently truncate input.
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub const CONTRACT: &str = "series-lite-edit/v1";
pub const SKILL_VERSION: &str = "20261027-v0.1.2-usage-reuse";
pub const SOURCE_COMMIT: &str = "6dea8d6c57dde38f04a4d71f05f4c039004dd4a9";
pub const SOURCE_LITE_EDIT_SHA256: &str =
    "8cd6ad5fb80f9a16c4dfce5101a09d04a41ea166273e4234db51f980f1329e6e";
pub const SOURCE_CONTEXT_PACKET_SHA256: &str =
    "698171410ca3be6a74ef6afabeb585e044228666bc8e9cd5d94ebcdf5ddd5275";
pub const SOURCE_VISUAL_BASELINE_SHA256: &str =
    "74bfc35bea91ad026f93f98e178f5a4fa6181ae41d895b323290ffcb10ea9109";

type Result<T> = std::result::Result<T, String>;
const CREATIVE: &[&str] = &["plan", "selection", "draft"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number { canonical: String, integer: bool },
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub(crate) fn object(&self) -> Result<&BTreeMap<String, Json>> {
        match self {
            Self::Object(v) => Ok(v),
            _ => Err("expected JSON object".into()),
        }
    }
    pub(crate) fn array(&self) -> Result<&[Json]> {
        match self {
            Self::Array(v) => Ok(v),
            _ => Err("expected JSON array".into()),
        }
    }
    pub(crate) fn string(&self) -> Result<&str> {
        match self {
            Self::String(v) => Ok(v),
            _ => Err("expected JSON string".into()),
        }
    }
    pub(crate) fn get(&self, key: &str) -> Result<&Json> {
        self.object()?
            .get(key)
            .ok_or_else(|| format!("missing field: {key}"))
    }
    pub(crate) fn is_integer(&self) -> bool {
        matches!(self, Self::Number { integer: true, .. })
    }
    pub(crate) fn integer(&self) -> Option<i128> {
        match self {
            Self::Number {
                canonical,
                integer: true,
            } => canonical.parse().ok(),
            _ => None,
        }
    }
}

fn need(ok: bool, error: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(error.into())
    }
}

pub(crate) fn fields(value: &Json, required: &[&str], optional: &[&str]) -> Result<()> {
    let map = value.object()?;
    need(
        required.iter().all(|k| map.contains_key(*k))
            && map
                .keys()
                .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str())),
        "invalid object fields",
    )
}

struct ContainerSeed {
    depth: usize,
    object: bool,
}
impl<'de> DeserializeSeed<'de> for ContainerSeed {
    type Value = Json;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> std::result::Result<Json, D::Error> {
        struct ContainerVisitor {
            depth: usize,
        }
        impl<'de> Visitor<'de> for ContainerVisitor {
            type Value = Json;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("strict JSON container")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Json, A::Error> {
                let mut out = BTreeMap::new();
                while let Some(key) = access.next_key::<String>()? {
                    if out.contains_key(&key) {
                        return Err(A::Error::custom("duplicate JSON key"));
                    }
                    let value = access.next_value::<Box<RawValue>>()?;
                    out.insert(
                        key,
                        parse_value(value.get(), self.depth + 1).map_err(A::Error::custom)?,
                    );
                }
                Ok(Json::Object(out))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Json, A::Error> {
                let mut out = Vec::new();
                while let Some(value) = access.next_element::<Box<RawValue>>()? {
                    out.push(parse_value(value.get(), self.depth + 1).map_err(A::Error::custom)?);
                }
                Ok(Json::Array(out))
            }
        }
        let visitor = ContainerVisitor { depth: self.depth };
        if self.object {
            d.deserialize_map(visitor)
        } else {
            d.deserialize_seq(visitor)
        }
    }
}

pub(crate) fn parse(raw: &[u8]) -> Result<Json> {
    let text = std::str::from_utf8(raw).map_err(|_| "invalid strict UTF-8 JSON")?;
    parse_value(text, 0)
}

fn parse_value(text: &str, depth: usize) -> Result<Json> {
    need(depth <= 128, "JSON nesting exceeds safe depth")?;
    // RawValue validates the complete JSON grammar without rounding numbers.
    let raw: Box<RawValue> = serde_json::from_str(text).map_err(|_| "invalid strict UTF-8 JSON")?;
    let text = raw.get();
    match text.as_bytes().first().copied() {
        Some(b'{') | Some(b'[') => {
            let mut d = serde_json::Deserializer::from_str(text);
            let value = ContainerSeed {
                depth,
                object: text.starts_with('{'),
            }
            .deserialize(&mut d)
            .map_err(|_| "invalid strict UTF-8 JSON or duplicate key")?;
            d.end().map_err(|_| "trailing JSON input")?;
            Ok(value)
        }
        Some(b'"') => serde_json::from_str::<String>(text)
            .map(Json::String)
            .map_err(|_| "invalid strict UTF-8 JSON string".into()),
        Some(b'n') => Ok(Json::Null),
        Some(b't') => Ok(Json::Bool(true)),
        Some(b'f') => Ok(Json::Bool(false)),
        _ => {
            let integer = !text.contains(['.', 'e', 'E']);
            let canonical = if integer {
                if text == "-0" {
                    "0".into()
                } else {
                    text.into()
                }
            } else {
                decimal(text)?
            };
            Ok(Json::Number { canonical, integer })
        }
    }
}

/// Python Decimal.__str__: preserve significant trailing zeroes, use exponent
/// notation for positive scales or adjusted exponents below -6.
fn decimal(text: &str) -> Result<String> {
    let negative = text.starts_with('-');
    let body = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (
            &body[..at],
            body[at + 1..]
                .parse::<i64>()
                .map_err(|_| "decimal exponent out of range")?,
        ),
        None => (body, 0),
    };
    let fraction = mantissa.split_once('.').map(|(_, v)| v.len()).unwrap_or(0);
    let joined = mantissa.replace('.', "");
    let coefficient = joined.trim_start_matches('0');
    let coefficient = if coefficient.is_empty() {
        "0"
    } else {
        coefficient
    };
    let scale = exponent
        .checked_sub(fraction as i64)
        .ok_or("decimal exponent out of range")?;
    let adjusted = scale
        .checked_add(coefficient.len() as i64 - 1)
        .ok_or("decimal exponent out of range")?;
    let mut out = if negative {
        "-".to_owned()
    } else {
        String::new()
    };
    if scale <= 0 && adjusted >= -6 {
        let split = coefficient.len() as i64 + scale;
        if scale == 0 {
            out.push_str(coefficient);
        } else if split > 0 {
            out.push_str(&coefficient[..split as usize]);
            out.push('.');
            out.push_str(&coefficient[split as usize..]);
        } else {
            out.push_str("0.");
            for _ in 0..-split {
                out.push('0');
            }
            out.push_str(coefficient);
        }
    } else {
        out.push(coefficient.as_bytes()[0] as char);
        if coefficient.len() > 1 {
            out.push('.');
            out.push_str(&coefficient[1..]);
        }
        out.push('E');
        if adjusted >= 0 {
            out.push('+');
        }
        out.push_str(&adjusted.to_string());
    }
    Ok(out)
}

fn encode(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Json::Number { canonical, .. } => out.push_str(canonical),
        Json::String(v) => out.push_str(&serde_json::to_string(v).expect("UTF-8 string")),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode(item, out);
            }
            out.push(']');
        }
        Json::Object(items) => {
            out.push('{');
            for (i, (key, value)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("UTF-8 key"));
                out.push(':');
                encode(value, out);
            }
            out.push('}');
        }
    }
}

pub(crate) fn canonical(value: &Json) -> Vec<u8> {
    let mut out = String::new();
    encode(value, &mut out);
    out.into_bytes()
}
pub fn sha256(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}

fn load_base(raw: &[u8], expected: &str) -> Result<Json> {
    need(
        hash_valid(expected),
        "base SHA-256 must be 64 lowercase hexadecimal characters",
    )?;
    need(
        sha256(raw) == expected,
        "base SHA-256 mismatch; rebuild from the current base",
    )?;
    let base = parse(raw)?;
    need(
        base.get("version")?.string()? == "series-storyboard-bundle-v1",
        "base must be an existing storyboard bundle",
    )?;
    let episodes = base.get("episodes")?.array()?;
    need(episodes.len() == 1, "base must contain exactly one episode")?;
    need(
        episodes[0].get("number")?.is_integer(),
        "base episode number must be an integer",
    )?;
    for field in ["assets", "scope"] {
        base.get(field)?.array()?;
    }
    for field in ["source", "settings"] {
        episodes[0].get(field)?.object()?;
    }
    episodes[0].get("scenes")?.array()?;
    Ok(base)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditResult {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub base_sha256: String,
    pub changed: bool,
    pub validation: &'static str,
}

fn finish(base_raw: &[u8], base: &Json, result: &Json) -> EditResult {
    let output = canonical(result);
    let changed = output != canonical(base);
    let bytes = if changed { output } else { base_raw.to_vec() };
    EditResult {
        sha256: sha256(&bytes),
        base_sha256: sha256(base_raw),
        bytes,
        changed,
        validation: "not-run",
    }
}

pub fn compose_bytes(
    base_raw: &[u8],
    expected_sha256: &str,
    response_raw: &[u8],
) -> Result<EditResult> {
    let base = load_base(base_raw, expected_sha256)?;
    let response = parse(response_raw)?;
    fields(&response, CREATIVE, &[])?;
    for field in CREATIVE {
        response.get(field)?.object()?;
    }
    let mut result = base.clone();
    for field in CREATIVE {
        *get_mut(
            &mut result,
            &["episodes".into(), "0".into(), (*field).into()],
            true,
        )? = response.get(field)?.clone();
    }
    Ok(finish(base_raw, &base, &result))
}

#[path = "series_lite_derive_plan.rs"]
mod derive_plan;

/// Explicit v0.4.1 assembly mode. Historical compose_bytes remains unchanged;
/// deriving indices never proves prior capacity planning or content quality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedEditResult {
    pub edit: EditResult,
    pub derived_bookkeeping: bool,
    pub bookkeeping_basis: &'static str,
}

pub fn compose_derived_bytes(
    base_raw: &[u8],
    expected_sha256: &str,
    response_raw: &[u8],
) -> Result<DerivedEditResult> {
    let edit = compose_bytes(base_raw, expected_sha256, response_raw)?;
    let base = load_base(base_raw, expected_sha256)?;
    let mut result = parse(&edit.bytes)?;
    let derived_bookkeeping = derive_plan::derive(&mut result)?;
    Ok(DerivedEditResult {
        edit: if derived_bookkeeping {
            finish(base_raw, &base, &result)
        } else {
            edit
        },
        derived_bookkeeping,
        bookkeeping_basis: "retrospective completed-draft indices; not prior capacity planning",
    })
}

/// Explicit new-route finishing mode. The caller must bind this mode to its
/// frozen 0.4.1 controller contract. Before applying any patch, prove that both
/// existing program-owned indices equal an exact retrospective derivation of
/// the base draft. Historical/explicit authored plans use patch_bytes instead.
/// Source/settings/assets remain protected by the unchanged patch contract.
pub fn patch_derived_bytes(base_raw: &[u8], patch_raw: &[u8]) -> Result<DerivedEditResult> {
    let edit = patch_bytes(base_raw, patch_raw)?;
    let base = parse(base_raw)?;
    let original_plan = base.get("episodes")?.array()?[0].get("plan")?;
    for key in ["workPackages", "provisionalUnits"] {
        original_plan.get(key)?.array()?;
    }
    let mut recalculated = base.clone();
    derive_plan::clear_indices(&mut recalculated)?;
    derive_plan::derive(&mut recalculated)?;
    let expected_plan = recalculated.get("episodes")?.array()?[0].get("plan")?;
    need(
        ["workPackages", "provisionalUnits"].iter().all(|key|
            canonical(original_plan.get(key).expect("checked index"))
                == canonical(expected_plan.get(key).expect("derived index"))),
        "derive-plan: base indices are not the program-derived indices of this draft",
    )?;
    let request = parse(patch_raw)?;
    for operation in request.get("operations")?.array()? {
        let path = tokens(operation.get("path")?.string()?)?;
        need(
            !(path.len() >= 4 && path[2] == "plan"
                && ["workPackages", "provisionalUnits"].contains(&path[3].as_str())),
            "derive-plan: program-owned indices cannot be patched directly",
        )?;
    }
    let mut result = parse(&edit.bytes)?;
    derive_plan::clear_indices(&mut result)?;
    derive_plan::derive(&mut result)?;
    Ok(DerivedEditResult {
        edit: finish(base_raw, &base, &result),
        derived_bookkeeping: true,
        bookkeeping_basis: "verified program-owned base indices; refreshed retrospective completed-draft indices; not prior capacity planning",
    })
}

fn tokens(path: &str) -> Result<Vec<String>> {
    need(path.starts_with('/'), "path must be a JSON Pointer")?;
    let mut out = Vec::new();
    for token in path[1..].split('/') {
        let mut chars = token.chars();
        let mut value = String::new();
        while let Some(c) = chars.next() {
            value.push(if c == '~' {
                match chars.next() {
                    Some('0') => '~',
                    Some('1') => '/',
                    _ => return Err("invalid JSON Pointer escape".into()),
                }
            } else {
                c
            });
        }
        out.push(value);
    }
    need(
        out.len() >= 3
            && out[0] == "episodes"
            && out[1] == "0"
            && CREATIVE.contains(&out[2].as_str()),
        "patch may only replace /episodes/0/{plan,selection,draft}",
    )?;
    Ok(out)
}

fn index(token: &str, len: usize) -> Result<usize> {
    need(
        token == "0"
            || (!token.starts_with('0')
                && !token.is_empty()
                && token.bytes().all(|v| v.is_ascii_digit())),
        "JSON Pointer array index must be a canonical nonnegative integer",
    )?;
    token
        .parse::<usize>()
        .ok()
        .filter(|v| *v < len)
        .ok_or_else(|| "JSON Pointer array index out of bounds".into())
}
fn get<'a>(mut value: &'a Json, path: &[String]) -> Result<&'a Json> {
    for token in path {
        value = match value {
            Json::Object(v) => v.get(token).ok_or("JSON Pointer object key not found")?,
            Json::Array(v) => &v[index(token, v.len())?],
            _ => return Err("JSON Pointer cannot descend into scalar".into()),
        };
    }
    Ok(value)
}
fn get_mut<'a>(value: &'a mut Json, path: &[String], allow_last: bool) -> Result<&'a mut Json> {
    if path.is_empty() {
        return Ok(value);
    }
    match value {
        Json::Object(v) => {
            if allow_last && path.len() == 1 {
                return Ok(v.entry(path[0].clone()).or_insert(Json::Null));
            }
            get_mut(
                v.get_mut(&path[0])
                    .ok_or("JSON Pointer object key not found")?,
                &path[1..],
                allow_last,
            )
        }
        Json::Array(v) => {
            let at = index(&path[0], v.len())?;
            get_mut(&mut v[at], &path[1..], allow_last)
        }
        _ => Err("JSON Pointer cannot descend into scalar".into()),
    }
}

pub fn patch_bytes(base_raw: &[u8], patch_raw: &[u8]) -> Result<EditResult> {
    let request = parse(patch_raw)?;
    fields(&request, &["baseSha256", "operations"], &[])?;
    let base = load_base(base_raw, request.get("baseSha256")?.string()?)?;
    let mut patches: Vec<(Vec<String>, Json)> = Vec::new();
    for op in request.get("operations")?.array()? {
        fields(op, &["op", "path", "expected", "value"], &[])?;
        need(
            op.get("op")?.string()? == "set",
            "only set operations are supported",
        )?;
        let path = tokens(op.get("path")?.string()?)?;
        need(
            !patches
                .iter()
                .any(|(other, _)| path.starts_with(other) || other.starts_with(&path)),
            "overlapping patch operations",
        )?;
        need(
            canonical(get(&base, &path)?) == canonical(op.get("expected")?),
            "expected value mismatch at patch operation",
        )?;
        patches.push((path, op.get("value")?.clone()));
    }
    let mut result = base.clone();
    for (path, value) in patches {
        *get_mut(&mut result, &path, false)? = value;
    }
    Ok(finish(base_raw, &base, &result))
}

fn nonempty(value: &Json) -> Result<&str> {
    let value = value.string()?;
    need(
        !value.trim().is_empty() && !value.contains(['\0', '\r']),
        "visual baseline: nonempty text required",
    )?;
    Ok(value)
}
pub(crate) fn visual_baseline(value: &Json) -> Result<()> {
    fields(
        value,
        &[
            "version",
            "id",
            "revision",
            "status",
            "visualStyle",
            "coverage",
        ],
        &["adoptionBasis"],
    )?;
    need(
        value.get("version")?.string()? == "series-visual-baseline-v1",
        "visual baseline: unsupported version",
    )?;
    need(
        !nonempty(value.get("id")?)?.contains('\n'),
        "visual baseline: id must be one line",
    )?;
    need(
        value
            .get("revision")?
            .integer()
            .is_some_and(|v| (1..=9_007_199_254_740_991).contains(&v)),
        "visual baseline: revision must be a positive safe integer",
    )?;
    need(
        value.get("status")?.string()? == "adopted",
        "visual baseline: an adopted baseline is required",
    )?;
    let style = nonempty(value.get("visualStyle")?)?;
    let coverage = value.get("coverage")?;
    fields(
        coverage,
        &[
            "form",
            "rendering",
            "materials",
            "colorLighting",
            "aspectRatio",
            "localExceptions",
        ],
        &[],
    )?;
    let mut seen = HashSet::new();
    for excerpt in coverage.object()?.values() {
        let text = nonempty(excerpt)?;
        need(
            style.contains(text),
            "visual baseline: coverage excerpt not in visualStyle",
        )?;
        seen.insert(text);
    }
    need(
        seen.len() > 1,
        "visual baseline: one repeated label cannot supply six coverage dimensions",
    )?;
    nonempty(value.get("adoptionBasis")?)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightResult {
    pub sha256: String,
    pub validation: &'static str,
    pub next: &'static str,
}

pub fn preflight_bytes(base_raw: &[u8]) -> Result<PreflightResult> {
    let digest = sha256(base_raw);
    let base = load_base(base_raw, &digest)?;
    need(
        base.get("exportProfile")?.string()? == "lingmo-v7",
        "preflight: lingmo-v7 is required",
    )?;
    need(
        base.get("generationTextVersion")?.string()? == "voice-delivery-v1",
        "preflight: voice-delivery-v1 is required",
    )?;
    need(
        hash_valid(base.get("sourceLockHash")?.string()?),
        "preflight: independent sourceLockHash is required",
    )?;
    let settings = base.get("episodes")?.array()?[0].get("settings")?;
    for key in ["visualStyle", "prohibitions"] {
        need(
            !settings.get(key)?.string()?.trim().is_empty(),
            "preflight: visualStyle and prohibitions must be nonempty strings",
        )?;
    }
    need(
        matches!(settings.get("maxUnitSeconds")?.integer(), Some(15 | 30)),
        "preflight: maxUnitSeconds must be 15 or 30",
    )?;
    need(
        ["off", "recommended", "all", "review"].contains(&settings.get("fantasyMode")?.string()?),
        "preflight: a valid fantasyMode is required",
    )?;
    let baseline = settings.get("visualBaseline")?;
    visual_baseline(baseline)?;
    need(
        settings.get("visualStyle")? == baseline.get("visualStyle")?,
        "preflight: visualStyle differs from the adopted baseline",
    )?;
    Ok(PreflightResult {
        sha256: digest,
        validation: "frozen-setup-fields-only",
        next: "source-lock contents, registry, completed draft and semantic review still required",
    })
}

/// Pure publication precondition, not a filesystem writer. The caller must
/// resolve paths inside its project root, reject symlinks, recheck the base hash
/// immediately before an exclusive create, and never overwrite an input file.
pub fn validate_output_target(
    base: &std::path::Path,
    response: &std::path::Path,
    output: &std::path::Path,
    exists: bool,
) -> Result<()> {
    need(
        !exists && output != base && output != response,
        "output already exists or aliases an input; refusing overwrite",
    )
}

#[cfg(test)]
#[path = "series_lite_edit_tests.rs"]
mod tests;
