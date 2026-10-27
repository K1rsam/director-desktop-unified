//! JSON.parse/JSON.stringify order for source-lock and published manifest bytes.
//! Numeric semantics intentionally match Node binary64, not creative Decimal edits.
use super::*;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;
#[derive(Clone, Debug)]
pub(super) enum Ordered {
    Scalar(Value),
    Array(Vec<Ordered>),
    Object(Vec<(String, Ordered)>),
}
impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "JSON")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Ordered, E> {
                Ok(Ordered::Scalar(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Ordered, E> {
                Ok(Ordered::Scalar(json!(v)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Ordered, E> {
                Ok(Ordered::Scalar(json!(v)))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Ordered, E> {
                Ok(Ordered::Scalar(json!(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Ordered, E> {
                Ok(Ordered::Scalar(json!(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Ordered, E> {
                self.visit_f64(v as f64)
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Ordered, E> {
                self.visit_f64(v as f64)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Ordered, A::Error> {
                let mut out = vec![];
                while let Some(v) = a.next_element()? {
                    out.push(v);
                }
                Ok(Ordered::Array(out))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Ordered, A::Error> {
                let mut out: Vec<(String, Ordered)> = vec![];
                while let Some((k, v)) = a.next_entry()? {
                    if let Some(old) = out.iter_mut().find(|(key, _)| *key == k) {
                        old.1 = v;
                    } else {
                        out.push((k, v));
                    }
                }
                Ok(Ordered::Object(out))
            }
        }
        d.deserialize_any(V)
    }
}
impl Ordered {
    pub(super) fn get(&self, key: &str) -> Option<&Self> {
        if let Self::Object(fields) = self {
            fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }
    pub(super) fn value(&self) -> Result<Value> {
        serde_json::from_str(&self.stringify(false)?).map_err(|e| e.to_string())
    }
    pub(super) fn from_value(value: &Value) -> Self {
        match value {
            Value::Array(a) => Self::Array(a.iter().map(Self::from_value).collect()),
            Value::Object(o) => Self::Object(
                o.iter()
                    .map(|(k, v)| (k.clone(), Self::from_value(v)))
                    .collect(),
            ),
            _ => Self::Scalar(value.clone()),
        }
    }
    pub(super) fn object(fields: Vec<(&str, Value)>) -> Self {
        Self::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k.into(), Self::from_value(&v)))
                .collect(),
        )
    }
    pub(super) fn stringify(&self, pretty: bool) -> Result<String> {
        self.render(pretty, 0)
    }
    fn render(&self, pretty: bool, depth: usize) -> Result<String> {
        match self {
            Self::Scalar(v) => crate::series_lite_registry::canonical(v),
            Self::Array(values) => {
                let parts = values
                    .iter()
                    .map(|v| v.render(pretty, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                Ok(join_parts("[", "]", parts, pretty, depth))
            }
            Self::Object(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_by_key(|(k, _)| index_key(k));
                let parts = fields
                    .into_iter()
                    .map(|(k, v)| {
                        Ok(format!(
                            "{}:{}{}",
                            serde_json::to_string(k).map_err(|e| e.to_string())?,
                            if pretty { " " } else { "" },
                            v.render(pretty, depth + 1)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(join_parts("{", "}", parts, pretty, depth))
            }
        }
    }
}
fn index_key(s: &str) -> (bool, u32) {
    match s.parse::<u32>() {
        Ok(n) if n < u32::MAX && n.to_string() == s => (false, n),
        _ => (true, 0),
    }
}
fn join_parts(open: &str, close: &str, parts: Vec<String>, pretty: bool, depth: usize) -> String {
    if parts.is_empty() {
        return format!("{open}{close}");
    }
    if !pretty {
        return format!("{open}{}{close}", parts.join(","));
    }
    let indent = "  ".repeat(depth + 1);
    format!(
        "{open}\n{indent}{}\n{}{close}",
        parts.join(&format!(",\n{indent}")),
        "  ".repeat(depth)
    )
}
