//! ECMAScript JSON serialization used by upstream module 22. Raw bytes remain
//! separate evidence; this module never reserializes an immutable source file.
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fmt;

fn number(n: f64) -> Result<String, String> {
    if !n.is_finite() {
        return Err("MODULE22_JSON_NUMBER".into());
    }
    Ok(ryu_js::Buffer::new().format_finite(n).to_owned())
}
fn index(key: &str) -> Option<u32> {
    key.parse::<u32>()
        .ok()
        .filter(|n| *n != u32::MAX && n.to_string() == key)
}
fn object(mut fields: Vec<(String, String)>, stable: bool) -> String {
    // JS enumerates integer-index properties before other keys even after
    // Object.fromEntries(Object.keys(value).sort()). Nonindices use UTF-16.
    fields.sort_by(|(a, _), (b, _)| match (index(a), index(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ if stable => a.encode_utf16().cmp(b.encode_utf16()),
        _ => std::cmp::Ordering::Equal,
    });
    format!(
        "{{{}}}",
        fields
            .into_iter()
            .map(|(k, v)| format!("{}:{v}", serde_json::to_string(&k).unwrap()))
            .collect::<Vec<_>>()
            .join(",")
    )
}
pub(crate) fn stable(value: &Value) -> Result<String, String> {
    Ok(match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).map_err(|e| e.to_string())?,
        Value::Number(v) => number(v.as_f64().ok_or("MODULE22_JSON_NUMBER")?)?,
        Value::Array(v) => format!(
            "[{}]",
            v.iter()
                .map(stable)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ),
        Value::Object(v) => object(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), stable(v)?)))
                .collect::<Result<_, String>>()?,
            true,
        ),
    })
}
pub(crate) fn hash(value: &Value) -> Result<String, String> {
    Ok(crate::pipeline::digest(&stable(value)?))
}
struct Seed;
impl<'de> DeserializeSeed<'de> for Seed {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<String, D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON data")
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<String, E> {
        Ok("null".into())
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<String, E> {
        Ok(v.to_string())
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> {
        serde_json::to_string(v).map_err(E::custom)
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<String, E> {
        self.visit_str(&v)
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<String, E> {
        number(v as f64).map_err(E::custom)
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<String, E> {
        number(v as f64).map_err(E::custom)
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<String, E> {
        number(v).map_err(E::custom)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<String, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element_seed(Seed)? {
            out.push(v);
        }
        Ok(format!("[{}]", out.join(",")))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<String, A::Error> {
        let mut out: Vec<(String, String)> = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            let v = map.next_value_seed(Seed)?;
            if let Some(old) = out.iter_mut().find(|(key, _)| key == &k) {
                old.1 = v;
            } else {
                out.push((k, v));
            }
        }
        Ok(object(out, false))
    }
}
pub(crate) fn stringify_raw(bytes: &[u8]) -> Result<String, String> {
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let value = Seed.deserialize(&mut d).map_err(|e| e.to_string())?;
    d.end().map_err(|e| e.to_string())?;
    Ok(value)
}
