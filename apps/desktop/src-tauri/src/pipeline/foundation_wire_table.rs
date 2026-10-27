//! Opt-in lossless catalogue representation AFTER typed transport-ID aliasing.
//! No semantic filtering, text rewriting, identity merging or side effects.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const PROFILE: &str = "foundation-catalogue-table/v1";
pub(crate) const FIELD: &str = "foundationCatalogueWire";
const FORMAT: &str = "FOUNDATION_CATALOGUE_TABLE_V1";
const GUIDE:&str="registry为无损表：rows每行首整数选assetShapes字段列表，后续值逐列对应；variants内每行首整数选variantShapes。形象preparation整数指向preparations完整原值。缺列表示字段缺省，null仍是显式null。行序、每个资产/形象身份、全部名称/别名/制作来源及说明保持，不合并同名。此表只省重复字段，不能省略任何已登记身份或改写值。以上仅描述输入表示；输出仍按原schema交本批差异，不回传此表或重交全量目录。";
type Result<T> = std::result::Result<T, String>;
fn sha(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn need(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("FOUNDATION_CATALOGUE_TABLE_INVALID: {message}"))
    }
}
fn index(value: &Value, len: usize) -> Result<usize> {
    let n = value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: invalid index")?;
    need(n < len, "index out of range")?;
    Ok(n)
}
fn shape_id(shapes: &mut Vec<Value>, columns: Value) -> usize {
    if let Some(n) = shapes.iter().position(|s| s == &columns) {
        n
    } else {
        shapes.push(columns);
        shapes.len() - 1
    }
}
fn value_id(values: &mut Vec<Value>, value: &Value) -> usize {
    if let Some(n) = values.iter().position(|s| s == value) {
        n
    } else {
        values.push(value.clone());
        values.len() - 1
    }
}
struct Encoder {
    assets: Vec<Value>,
    variants: Vec<Value>,
    preparations: Vec<Value>,
}
impl Encoder {
    fn row(&mut self, object: &Value, asset: bool) -> Result<Value> {
        let object = object
            .as_object()
            .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: catalogue row must be object")?;
        let columns = json!(object.keys().collect::<Vec<_>>());
        let id = shape_id(
            if asset {
                &mut self.assets
            } else {
                &mut self.variants
            },
            columns,
        );
        let mut row = vec![json!(id)];
        for (k, v) in object {
            row.push(if asset && k == "variants" && v.is_array() {
                json!(v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| self.row(v, false))
                    .collect::<Result<Vec<_>>>()?)
            } else if !asset && k == "preparation" {
                json!(value_id(&mut self.preparations, v))
            } else {
                v.clone()
            });
        }
        Ok(json!(row))
    }
}
/// Preserves every field (including future/unknown fields), explicit null and
/// missing fields. Only declared asset/variant slots get structural encoding.
pub(crate) fn encode_catalogue(catalogue: &Value) -> Result<Value> {
    let assets = catalogue
        .as_array()
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: registry must be array")?;
    let mut e = Encoder {
        assets: vec![],
        variants: vec![],
        preparations: vec![],
    };
    let rows = assets
        .iter()
        .map(|a| e.row(a, true))
        .collect::<Result<Vec<_>>>()?;
    let table = json!({"format":FORMAT,"interpretation":GUIDE,"catalogueSha256":sha(catalogue),"assetShapes":e.assets,"variantShapes":e.variants,"preparations":e.preparations,"rows":rows});
    need(
        decode_catalogue(&table)? == *catalogue,
        "roundtrip mismatch",
    )?;
    Ok(table)
}
pub(crate) fn pack_registry(catalogue: &Value) -> Result<Value> {
    encode_catalogue(catalogue)
}
pub(crate) fn unpack_registry(table: &Value) -> Result<Value> {
    decode_catalogue(table)
}
fn shapes<'a>(table: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    let rows = table[key]
        .as_array()
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: shapes missing")?;
    let mut seen = std::collections::HashSet::new();
    for columns in rows {
        let columns = columns
            .as_array()
            .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: columns missing")?;
        let mut previous: Option<&str> = None;
        for column in columns {
            let text = column
                .as_str()
                .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: column must be string")?;
            need(
                previous.is_none_or(|p| p < text),
                "columns must be distinct and sorted",
            )?;
            previous = Some(text)
        }
        need(seen.insert(json!(columns).to_string()), "duplicate shape")?;
    }
    Ok(rows)
}
fn decode_row(
    row: &Value,
    asset: bool,
    asset_shapes: &[Value],
    variant_shapes: &[Value],
    preparations: &[Value],
) -> Result<Value> {
    let cells = row
        .as_array()
        .filter(|r| !r.is_empty())
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: invalid row")?;
    let shape = if asset { asset_shapes } else { variant_shapes };
    let columns = shape[index(&cells[0], shape.len())?].as_array().unwrap();
    need(
        cells.len() == columns.len() + 1,
        "row length differs from shape",
    )?;
    let mut out = serde_json::Map::new();
    for (column, v) in columns.iter().zip(&cells[1..]) {
        let k = column.as_str().unwrap();
        let value = if asset && k == "variants" && v.is_array() {
            json!(v
                .as_array()
                .unwrap()
                .iter()
                .map(|r| decode_row(r, false, asset_shapes, variant_shapes, preparations))
                .collect::<Result<Vec<_>>>()?)
        } else if !asset && k == "preparation" {
            preparations[index(v, preparations.len())?].clone()
        } else {
            v.clone()
        };
        out.insert(k.into(), value);
    }
    Ok(Value::Object(out))
}
pub(crate) fn decode_catalogue(table: &Value) -> Result<Value> {
    let object = table
        .as_object()
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: table missing")?;
    need(
        object.len() == 7
            && [
                "format",
                "interpretation",
                "catalogueSha256",
                "assetShapes",
                "variantShapes",
                "preparations",
                "rows",
            ]
            .iter()
            .all(|k| object.contains_key(*k)),
        "unknown or missing table field",
    )?;
    need(
        table["format"] == FORMAT && table["interpretation"] == GUIDE,
        "wrong table contract",
    )?;
    let asset_shapes = shapes(table, "assetShapes")?;
    let variant_shapes = shapes(table, "variantShapes")?;
    let preparations = table["preparations"]
        .as_array()
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: preparations missing")?;
    let rows = table["rows"]
        .as_array()
        .ok_or("FOUNDATION_CATALOGUE_TABLE_INVALID: rows missing")?;
    let value = json!(rows
        .iter()
        .map(|r| decode_row(r, true, asset_shapes, variant_shapes, preparations))
        .collect::<Result<Vec<_>>>()?);
    need(
        table["catalogueSha256"] == sha(&value),
        "catalogue digest mismatch",
    )?;
    Ok(value)
}
/// Caller must execute typed-ID replacement first. A marker-less frozen input
/// remains exactly unchanged. New input marker cannot silently name a fallback.
pub(crate) fn pack_input(input: &mut Value) -> Result<bool> {
    let Some(marker) = input.get(FIELD) else {
        return Ok(false);
    };
    need(marker == PROFILE, "unknown wire profile")?;
    let Some(registry) = input.get("registry") else {
        return Ok(false);
    };
    if registry.get("format") == Some(&json!(FORMAT)) {
        decode_catalogue(registry)?;
        return Ok(false);
    }
    let table = encode_catalogue(registry)?;
    if table.to_string().encode_utf16().count() >= registry.to_string().encode_utf16().count() {
        return Ok(false);
    }
    input["registry"] = table;
    Ok(true)
}

#[cfg(test)]
#[path = "foundation_wire_table_tests.rs"]
mod tests;
