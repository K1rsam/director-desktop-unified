//! Lossless, task-local ID aliases for the foundation transport. Canonical IDs
//! and source text stay in the frozen request; this module has no side effects.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) const PROFILE: &str = "foundation-compact/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum Kind {
    Episode,
    Scene,
    Row,
    Asset,
    Variant,
}
impl Kind {
    fn prefix(self) -> char {
        match self {
            Self::Episode => 'e',
            Self::Scene => 's',
            Self::Row => 'r',
            Self::Asset => 'a',
            Self::Variant => 'v',
        }
    }
}

#[derive(Debug)]
pub(crate) struct CompiledWire {
    pub(crate) input: Value,
    pub(crate) schema: Value,
    forward: HashMap<String, (Kind, String)>,
    reverse: HashMap<String, (Kind, String)>,
    canonical: HashSet<String>,
    conflict_decision: bool,
}

#[derive(Default)]
struct Ids {
    values: Vec<(Kind, String)>,
    known: HashMap<String, Kind>,
}
impl Ids {
    fn add(&mut self, kind: Kind, value: &Value) -> Result<(), String> {
        let Some(id) = value.as_str().filter(|s| !s.is_empty()) else {
            return Ok(());
        };
        if let Some(previous) = self.known.get(id) {
            if *previous != kind {
                return Err(format!("运输身份跨类型歧义: {id}"));
            }
        } else {
            self.known.insert(id.into(), kind);
            self.values.push((kind, id.into()));
        }
        Ok(())
    }
}

fn column_kind(column: &str) -> Option<Kind> {
    match column {
        "rowId" => Some(Kind::Row),
        "episodeId" => Some(Kind::Episode),
        "sceneId" => Some(Kind::Scene),
        _ => None,
    }
}

fn collect_table(
    ids: &mut Ids,
    columns: &Value,
    rows: &Value,
    dictionaries: Option<&Value>,
) -> Result<(), String> {
    let (Some(columns), Some(rows)) = (columns.as_array(), rows.as_array()) else {
        return Ok(());
    };
    for (index, column) in columns.iter().enumerate() {
        let Some(name) = column.as_str() else {
            continue;
        };
        let Some(kind) = column_kind(name) else {
            continue;
        };
        if let Some(dictionary) = dictionaries
            .and_then(|d| d.get(name))
            .and_then(Value::as_array)
        {
            for value in dictionary {
                ids.add(kind, value)?;
            }
        }
        for row in rows {
            if let Some(value) = row.get(index) {
                ids.add(kind, value)?;
            }
        }
    }
    Ok(())
}

fn collect_tables(ids: &mut Ids, value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if let (Some(columns), Some(rows)) = (map.get("columns"), map.get("rows")) {
                collect_table(ids, columns, rows, map.get("dictionaries"))?;
            }
            if let Some(columns) = map.get("backgroundColumns") {
                for name in ["background", "backgroundCandidates"] {
                    if let Some(rows) = map.get(name) {
                        collect_table(ids, columns, rows, None)?;
                    }
                }
            }
            if let (Some(columns), Some(rows)) = (
                map.get("historicalColumns"),
                map.get("historicalEvidenceCandidates"),
            ) {
                collect_table(ids, columns, rows, None)?;
            }
            for child in map.values() {
                collect_tables(ids, child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_tables(ids, child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// This traversal registers identities only at typed slots supplied by the
// program, never by a person's name or by an arbitrary string's spelling.
fn collect_structured(ids: &mut Ids, value: &Value, context: Option<Kind>) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            let context = match (context, map.get("kind").and_then(Value::as_str)) {
                (Some(_), Some("VARIANT")) => Some(Kind::Variant),
                (Some(_), Some("BASE")) => Some(Kind::Asset),
                _ => context,
            };
            for (field, child) in map {
                let direct = match field.as_str() {
                    "episodeId" | "episode_id" => Some(Kind::Episode),
                    "sceneId" | "scene_id" | "sceneKey" => Some(Kind::Scene),
                    "assetKey" | "assetId" | "baseKey" => Some(Kind::Asset),
                    "variantKey" | "variantId" | "baseVariantKey" => Some(Kind::Variant),
                    "key" => context,
                    _ => None,
                };
                if let Some(kind) = direct {
                    ids.add(kind, child)?;
                }
                if matches!(
                    field.as_str(),
                    "scope"
                        | "coverage"
                        | "episodes"
                        | "episodeIds"
                        | "episode_ids"
                        | "usageEpisodes"
                ) {
                    if let Some(values) = child.as_array() {
                        for value in values {
                            ids.add(Kind::Episode, value)?;
                        }
                    }
                }
                if matches!(field.as_str(), "sceneRefs" | "scenes") {
                    if let Some(rows) = child.as_array() {
                        for row in rows {
                            if row.is_array() {
                                if let Some(ep) = row.get(0) {
                                    ids.add(Kind::Episode, ep)?;
                                }
                                if let Some(scene) = row.get(1) {
                                    ids.add(Kind::Scene, scene)?;
                                }
                            }
                        }
                    }
                }
                let next = match field.as_str() {
                    "registry" | "candidates" | "assets" | "materialCandidates" => {
                        Some(Kind::Asset)
                    }
                    "variants" | "crossBatchVariants" => Some(Kind::Variant),
                    _ => None,
                };
                collect_structured(ids, child, next)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_structured(ids, child, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Only the new decision protocol gains these additional typed slots. Changing
// compilation of an older frozen compact request would invalidate its evidence.
fn collect_decision_ids(ids:&mut Ids,value:&Value)->Result<(),String> {
    match value {
        Value::Object(fields)=>for (name,child) in fields {
            match name.as_str() {
                "sourceAssetId"|"targetAssetId"=>ids.add(Kind::Asset,child)?,
                "sourceVariantId"|"targetVariantId"=>ids.add(Kind::Variant,child)?,
                _=>{},
            }
            collect_decision_ids(ids,child)?;
        },
        Value::Array(values)=>for child in values {collect_decision_ids(ids,child)?;},
        _=>{},
    }
    Ok(())
}

fn collect_decision_schema_ids(ids:&mut Ids,schema:&Value)->Result<(),String> {
    let Some(object)=schema.as_object() else{return Ok(())};
    if let Some(properties)=object.get("properties").and_then(Value::as_object) {
        for (name,child) in properties {
            let kind=match name.as_str(){"targetAssetId"=>Some(Kind::Asset),"targetVariantId"=>Some(Kind::Variant),_=>None};
            if let Some(kind)=kind {
                if let Some(values)=child.get("enum").and_then(Value::as_array) {
                    for value in values {ids.add(kind,value)?;}
                }
            }
            collect_decision_schema_ids(ids,child)?;
        }
    }
    for field in ["$defs","definitions"] {
        if let Some(children)=object.get(field).and_then(Value::as_object) {
            for child in children.values(){collect_decision_schema_ids(ids,child)?;}
        }
    }
    if let Some(child)=object.get("items"){collect_decision_schema_ids(ids,child)?;}
    for field in ["anyOf","oneOf","allOf","prefixItems"] {
        if let Some(children)=object.get(field).and_then(Value::as_array) {
            for child in children {collect_decision_schema_ids(ids,child)?;}
        }
    }
    Ok(())
}

fn short_kind(value: &str) -> Option<Kind> {
    let (head, tail) = value.split_at_checked(1)?;
    if tail.is_empty() || !tail.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    match head {
        "e" => Some(Kind::Episode),
        "s" => Some(Kind::Scene),
        "r" => Some(Kind::Row),
        "a" => Some(Kind::Asset),
        "v" => Some(Kind::Variant),
        _ => None,
    }
}
fn split_child(value: &str) -> Option<(&str, &str)> {
    let (base, line) = value.rsplit_once("__L")?;
    if line.is_empty()
        || !line.bytes().all(|c| c.is_ascii_digit())
        || line.parse::<usize>().ok()? == 0
    {
        return None;
    }
    Some((base, &value[base.len()..]))
}

impl CompiledWire {
    fn alias(&self, value: &str, expected: Option<Kind>, restore: bool) -> Result<String, String> {
        let map = if restore {
            &self.reverse
        } else {
            &self.forward
        };
        if let Some((kind, replacement)) = map.get(value) {
            if expected.is_some_and(|expected| expected != *kind) {
                return Err(format!("运输引用类型不匹配: {value}"));
            }
            return Ok(replacement.clone());
        }
        // Fixed source split naming is a coordinate contract, not fuzzy alias
        // matching. The ordinary source-bound validator still checks its cut.
        if expected == Some(Kind::Scene) {
            if let Some((base, suffix)) = split_child(value) {
                if let Some((kind, replacement)) = map.get(base) {
                    if *kind != Kind::Scene {
                        return Err(format!("拆场引用类型不匹配: {value}"));
                    }
                    return Ok(format!("{replacement}{suffix}"));
                }
                if restore && short_kind(base).is_some() && !self.canonical.contains(base) {
                    return Err(format!("未知短场引用: {value}"));
                }
            }
        }
        if restore {
            // The previous strict sourceRow schema used r:start:end. This is
            // the same supplied row, not a generated offset or fuzzy repair.
            if let Some((Kind::Row, alias)) =
                self.forward.get(value).filter(|_| value.starts_with("r:"))
            {
                if expected.is_some_and(|expected| expected != Kind::Row) {
                    return Err(format!("原文坐标引用类型不匹配: {value}"));
                }
                return Ok(self
                    .reverse
                    .get(alias)
                    .ok_or("原文坐标别名缺少还原身份")?
                    .1
                    .clone());
            }
            if self.canonical.contains(value) {
                if expected.is_some_and(|expected| {
                    self.forward
                        .get(value)
                        .is_some_and(|(kind, _)| *kind != expected)
                }) {
                    return Err(format!("canonical引用类型不匹配: {value}"));
                }
                return Ok(value.into());
            }
            if short_kind(value).is_some() {
                return Err(format!("未知短引用: {value}"));
            }
        }
        Ok(value.into())
    }
    fn scalar(
        &self,
        value: &mut Value,
        expected: Option<Kind>,
        restore: bool,
    ) -> Result<(), String> {
        if let Some(text) = value.as_str() {
            *value = Value::String(self.alias(text, expected, restore)?);
        }
        Ok(())
    }
    fn identity(&self, value: &mut Value, restore: bool) -> Result<(), String> {
        if let Some(id) = value.as_str() {
            let kind = if restore {
                self.reverse.get(id).or_else(|| self.forward.get(id))
            } else {
                self.forward.get(id)
            }
            .map(|(kind, _)| *kind)
            .or_else(|| short_kind(id));
            if kind.is_some_and(|kind| !matches!(kind, Kind::Asset | Kind::Variant)) {
                return Err(format!("素材来源引用类型不匹配: {id}"));
            }
        }
        self.scalar(value, None, restore)
    }
    fn list(&self, value: &mut Value, expected: Option<Kind>, restore: bool) -> Result<(), String> {
        if let Some(values) = value.as_array_mut() {
            for value in values {
                if value.is_string() {
                    self.scalar(value, expected, restore)?;
                } else {
                    self.walk(value, None, restore)?;
                }
            }
        }
        Ok(())
    }
    fn pairs(&self, value: &mut Value, restore: bool) -> Result<(), String> {
        if let Some(values) = value.as_array_mut() {
            for value in values {
                if let Some(pair) = value.as_array_mut() {
                    if let Some(ep) = pair.get_mut(0) {
                        self.scalar(ep, Some(Kind::Episode), restore)?;
                    }
                    if let Some(scene) = pair.get_mut(1) {
                        self.scalar(scene, Some(Kind::Scene), restore)?;
                    }
                } else {
                    self.walk(value, None, restore)?;
                }
            }
        }
        Ok(())
    }
    fn table(&self, columns: &Value, rows: &mut Value, restore: bool) -> Result<(), String> {
        let (Some(columns), Some(rows)) = (columns.as_array(), rows.as_array_mut()) else {
            return Ok(());
        };
        for row in rows {
            let Some(row) = row.as_array_mut() else {
                continue;
            };
            for (index, column) in columns.iter().enumerate() {
                let Some(name) = column.as_str() else {
                    continue;
                };
                if let Some(value) = row.get_mut(index) {
                    if let Some(kind) = column_kind(name) {
                        self.scalar(value, Some(kind), restore)?;
                    } else if name == "anchorRowIds" {
                        self.list(value, Some(Kind::Row), restore)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn walk(&self, value: &mut Value, context: Option<Kind>, restore: bool) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                let context = match (context, map.get("kind").and_then(Value::as_str)) {
                    (Some(_), Some("VARIANT")) => Some(Kind::Variant),
                    (Some(_), Some("BASE")) => Some(Kind::Asset),
                    _ => context,
                };
                // Table rows are interpreted by their declared columns. Text,
                // claim, speakers, ranges and integer dictionary indexes stay.
                if let Some(columns) = map.get("columns").cloned() {
                    if let Some(rows) = map.get_mut("rows") {
                        self.table(&columns, rows, restore)?;
                    }
                    if let Some(dictionaries) =
                        map.get_mut("dictionaries").and_then(Value::as_object_mut)
                    {
                        for (column, values) in dictionaries {
                            if let Some(kind) = column_kind(column) {
                                self.list(values, Some(kind), restore)?;
                            }
                        }
                    }
                }
                if let Some(columns) = map.get("backgroundColumns").cloned() {
                    for field in ["background", "backgroundCandidates"] {
                        if let Some(rows) = map.get_mut(field) {
                            self.table(&columns, rows, restore)?;
                        }
                    }
                }
                if let Some(columns) = map.get("historicalColumns").cloned() {
                    if let Some(rows) = map.get_mut("historicalEvidenceCandidates") {
                        if rows.is_array() {
                            self.table(&columns, rows, restore)?;
                        }
                    }
                }
                for (field, child) in map.iter_mut() {
                    match field.as_str() {
                        "episodeId" | "episode_id" | "episode" => {
                            self.scalar(child, Some(Kind::Episode), restore)?
                        }
                        "sceneId" | "scene_id" | "sceneKey" => {
                            self.scalar(child, Some(Kind::Scene), restore)?
                        }
                        "rowId" => self.scalar(child, Some(Kind::Row), restore)?,
                        "coordinate"
                            if child.as_str().is_some_and(|id| {
                                self.forward.contains_key(id) || self.reverse.contains_key(id)
                            }) =>
                        {
                            self.scalar(child, None, restore)?;
                        }
                        "sourceAssetId" | "targetAssetId" if self.conflict_decision => {
                            self.scalar(child, Some(Kind::Asset), restore)?
                        }
                        "targetVariantId" if self.conflict_decision => {
                            self.scalar(child, Some(Kind::Variant), restore)?
                        }
                        "assetId" | "assetKey" | "baseKey" | "asset_id" => {
                            self.scalar(child, Some(Kind::Asset), restore)?
                        }
                        "variantId" | "variantKey" | "baseVariantKey" | "sourceVariantId"
                        | "variant_id" | "source_variant_id" => {
                            self.scalar(child, Some(Kind::Variant), restore)?
                        }
                        "fromKey" => self.identity(child, restore)?,
                        "keepKey" | "intoKey" => self.scalar(child, Some(Kind::Asset), restore)?,
                        "key" if context.is_some() => self.scalar(child, context, restore)?,
                        "id" if self.conflict_decision && context.is_some() => {
                            self.scalar(child,context,restore)?
                        }
                        "key"
                            if child.as_str().is_some_and(|id| {
                                self.forward
                                    .get(id)
                                    .or_else(|| self.reverse.get(id))
                                    .is_some_and(|(kind, _)| {
                                        matches!(kind, Kind::Asset | Kind::Variant)
                                    })
                            }) =>
                        {
                            self.identity(child, restore)?
                        }
                        "scope" | "coverage" | "episodes" | "episodeIds" | "episode_ids"
                        | "usageEpisodes"
                            if child.is_array() =>
                        {
                            self.list(child, Some(Kind::Episode), restore)?
                        }
                        "evidenceRows" if self.conflict_decision => {
                            self.list(child, Some(Kind::Row), restore)?
                        }
                        "sourceRefs" | "anchorRowIds" | "evidenceRefs" => {
                            self.list(child, Some(Kind::Row), restore)?
                        }
                        "assetKeys"
                        | "subjectAssetKeys"
                        | "previousSubjectAssetKeys"
                        | "removeAssetKeys"
                        | "removeKeys" => self.list(child, Some(Kind::Asset), restore)?,
                        "variantKeys" => self.list(child, Some(Kind::Variant), restore)?,
                        "resolvedIssues" if child.is_array() => {
                            for item in child.as_array_mut().unwrap() {
                                if item.as_str().is_some_and(|id| {
                                    self.forward.contains_key(id) || self.reverse.contains_key(id)
                                }) {
                                    self.scalar(item, None, restore)?;
                                }
                            }
                        }
                        "sceneRefs" | "scenes" if child.is_array() => self.pairs(child, restore)?,
                        "registry" | "candidates" | "assets" | "materialCandidates" => {
                            self.walk(child, Some(Kind::Asset), restore)?
                        }
                        "current" | "candidate" if self.conflict_decision
                            && child.get("variants").is_some_and(Value::is_array)
                            && child.get("id").and_then(Value::as_str).is_some_and(|id|
                                self.forward.get(id).or_else(||self.reverse.get(id)).is_some_and(|(kind,_)|*kind==Kind::Asset)) => {
                            self.walk(child,Some(Kind::Asset),restore)?
                        }
                        "variants" | "crossBatchVariants" => {
                            self.walk(child, Some(Kind::Variant), restore)?
                        }
                        // Table payloads were already handled above; other
                        // plain arrays only recurse into objects, not strings.
                        "rows" | "background" | "backgroundCandidates" if child.is_array() => {}
                        "historicalEvidenceCandidates" if child.is_array() => {}
                        "dictionaries" => {}
                        _ => self.walk(child, None, restore)?,
                    }
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.walk(child, context, restore)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// Atomic and idempotent: rejected aliases do not partially mutate results.
    /// Already accepted canonical cache results may safely pass through again.
    pub(crate) fn restore(&self, value: &mut Value) -> Result<(), String> {
        let mut restored = value.clone();
        self.walk(&mut restored, None, true)?;
        *value = restored;
        Ok(())
    }

    /// Rewrite only explicitly typed decision schema slots. Hash enums,
    /// conflict IDs, paths, prose, and arbitrary enum strings stay byte-exact.
    fn decision_schema(&self, schema:&mut Value)->Result<(),String> {
        let Some(object)=schema.as_object_mut() else{return Ok(())};
        if let Some(properties)=object.get_mut("properties").and_then(Value::as_object_mut) {
            for (name,child) in properties {
                let kind=match name.as_str(){"targetAssetId"=>Some(Kind::Asset),"targetVariantId"=>Some(Kind::Variant),_=>None};
                if let Some(kind)=kind {
                    if let Some(values)=child.get_mut("enum").and_then(Value::as_array_mut) {
                        for value in values {self.scalar(value,Some(kind),false)?;}
                    }
                }
                self.decision_schema(child)?;
            }
        }
        for field in ["$defs","definitions"] {
            if let Some(children)=object.get_mut(field).and_then(Value::as_object_mut) {
                for child in children.values_mut(){self.decision_schema(child)?;}
            }
        }
        if let Some(child)=object.get_mut("items"){self.decision_schema(child)?;}
        for field in ["anyOf","oneOf","allOf","prefixItems"] {
            if let Some(children)=object.get_mut(field).and_then(Value::as_array_mut) {
                for child in children {self.decision_schema(child)?;}
            }
        }
        Ok(())
    }
}

pub(crate) fn compile(stage: &str, input: &Value, schema: &Value) -> Result<CompiledWire, String> {
    if !matches!(stage, "F2" | "Q1" | "foundation:F2" | "foundation:Q1") {
        return Err("紧凑运输仅适用于F2/Q1".into());
    }
    if input.get("modelInputProfile").and_then(Value::as_str) != Some(PROFILE) {
        return Err("未知或缺失第一阶段运输profile".into());
    }
    let mut ids = Ids::default();
    if let Some(scope) = input.get("scope").and_then(Value::as_array) {
        for id in scope {
            ids.add(Kind::Episode, id)?;
        }
    }
    // Current rows first, then background/historical rows. Ordering is stable
    // within the frozen request and never conflates same-name identities.
    if let Some(source) = input.get("source") {
        collect_tables(&mut ids, source)?;
    }
    collect_tables(&mut ids, input)?;
    collect_structured(&mut ids, input, None)?;
    let conflict_decision=input.get("foundationConflictResolution").is_some();
    if conflict_decision {
        collect_decision_ids(&mut ids,input)?;
        // Schema-only canonical a1/v1 are real IDs, not reusable wire aliases.
        // Register before allocation, including null-capable enum definitions.
        collect_decision_schema_ids(&mut ids,schema)?;
    }
    let canonical: HashSet<_> = ids.known.keys().cloned().collect();
    let mut forward = HashMap::new();
    let mut reverse = HashMap::new();
    let mut counts = HashMap::<Kind, usize>::new();
    for (kind, id) in ids.values {
        let index = counts.entry(kind).or_default();
        let alias = loop {
            *index += 1;
            let alias = format!("{}{index}", kind.prefix());
            if !canonical.contains(&alias)
                && !canonical
                    .iter()
                    .any(|id| id.starts_with(&format!("{alias}__L")))
            {
                break alias;
            }
        };
        forward.insert(id.clone(), (kind, alias.clone()));
        reverse.insert(alias, (kind, id));
    }
    if let Some(prefix) = input
        .get("sourceSha256")
        .and_then(Value::as_str)
        .and_then(|sha| sha.get(..12))
        .map(|sha| format!("row:{sha}:"))
    {
        let rows: Vec<_> = forward
            .iter()
            .filter(|(_, (kind, _))| *kind == Kind::Row)
            .map(|(id, (_, alias))| (id.clone(), alias.clone()))
            .collect();
        for (id, alias) in rows {
            if let Some(coordinates) = id.strip_prefix(&prefix) {
                let short = format!("r:{coordinates}");
                if let Some((previous_kind, previous_alias)) = forward.get(&short) {
                    if *previous_kind != Kind::Row || *previous_alias != alias {
                        return Err("原文坐标别名存在歧义".into());
                    }
                } else {
                    forward.insert(short, (Kind::Row, alias));
                }
            }
        }
    }
    let mut wire = CompiledWire {
        input: input.clone(),
        schema: schema.clone(),
        forward,
        reverse,
        canonical,
        conflict_decision,
    };
    // Keep borrows disjoint without changing the immutable mapping.
    let mut compiled = input.clone();
    wire.walk(&mut compiled, None, false)?;
    let guide=compiled["transportInstructions"].as_str().unwrap_or("").replace(
        "真正事项证据、方向与拆场rowId只选schema枚举的r:start:end，与输入第1列的row:源前缀:start:end逐个对应；程序绑定当前sourceSha还原，不重算或拼接start/end。",
        "真正事项证据、方向与拆场rowId只选schema枚举的rN，与输入第1列短行身份逐个对应；不猜编号或计算偏移。",
    );
    let prefix="仅按输入提供的短身份引用：eN集、sN场、rN原文行、aN资产、vN形象；sourceSha256照输入原值。名字/原文/quote/说明是逐字资料，不是身份。必要拆场沿已提供场身份加__L起始行号，仍须给真实完整原文anchor。";
    compiled["transportInstructions"] = json!(if guide.starts_with(prefix) {
        guide
    } else {
        format!("{prefix}{guide}")
    });
    wire.input = compiled;
    if let Some(rows) = wire
        .schema
        .pointer_mut("/$defs/sourceRow/enum")
        .and_then(Value::as_array_mut)
    {
        for value in rows {
            if let Some(id) = value.as_str() {
                let Some((Kind::Row, alias)) = wire.forward.get(id) else {
                    return Err(format!("schema引用未提供原文行: {id}"));
                };
                *value = json!(alias);
            }
        }
    }
    if input.get("foundationConflictResolution").is_some() {
        let mut schema=wire.schema.clone();
        wire.decision_schema(&mut schema)?;
        wire.schema=schema;
    }
    Ok(wire)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> Value {
        super::super::foundation_delivery::synthetic::fixture(name)
    }
    fn from_snapshot(snapshot: &Value) -> CompiledWire {
        let mut input = snapshot["input"].clone();
        input["modelInputProfile"] = json!(PROFILE);
        compile(
            snapshot["stage"].as_str().unwrap(),
            &input,
            &snapshot["schema"],
        )
        .unwrap()
    }
    #[test]
    fn synthetic_f2_q1_replies_and_inputs_roundtrip_without_text_changes() {
        for name in [
            "rejected-task.json",
            "rejected-q1.json",
            "rejected-q1-final.json",
        ] {
            let saved = fixture(name);
            let wire = from_snapshot(&saved["snapshot"]);
            let mut input = wire.input.clone();
            wire.restore(&mut input).unwrap();
            input["transportInstructions"] =
                saved["snapshot"]["input"]["transportInstructions"].clone();
            input.as_object_mut().unwrap().remove("modelInputProfile");
            assert_eq!(input, saved["snapshot"]["input"], "{name}");
            let mut canonical = saved["result"].clone();
            wire.restore(&mut canonical).unwrap();
            let mut response = canonical.clone();
            wire.walk(&mut response, None, false).unwrap();
            let compact_chars = response.to_string().encode_utf16().count();
            wire.restore(&mut response).unwrap();
            assert_eq!(response, canonical, "{name}");
            wire.restore(&mut response).unwrap();
            assert_eq!(response, canonical);
            println!(
                "{name}: input {} -> {}, response {} -> {}",
                saved["snapshot"]["input"]
                    .to_string()
                    .encode_utf16()
                    .count(),
                wire.input.to_string().encode_utf16().count(),
                canonical.to_string().encode_utf16().count(),
                compact_chars
            );
        }
    }
    fn simple() -> CompiledWire {
        let input = json!({"modelInputProfile":PROFILE,"sourceSha256":"0123456789abcdef","scope":["episode:long:1"],"source":{"columns":["rowId","line","episodeId","sceneId","kind","speaker","channel","start","end","text"],"rows":[["row:0123456789ab:0:8",1,"episode:long:1","scene:long:1","dialogue","e1","VO",0,8,"e1说r1"]]},"scenes":[["episode:long:1","scene:long:1","s1"]],"registry":[{"key":"asset:long:1","name":"e1","variants":[{"key":"variant:long:1","name":"r1"}]},{"key":"asset:long:2","name":"e1","variants":[]}],"candidates":[{"key":"asset:long:1","name":"e1","variants":[{"key":"variant:long:1","fromKey":"asset:long:2","sceneRefs":[["episode:long:1","scene:long:1"]]}]}],"evidence":{"historicalColumns":["rowId","episodeId","sceneId","text","claim","retrievalMode","anchorRowIds"],"historicalEvidenceCandidates":[["row:0123456789ab:8:16","episode:long:0","scene:long:0","e1 r1 a1","CHARACTER_CLAIM","SUBJECT_MENTION",["row:0123456789ab:8:16"]]]}});
        compile(
            "Q1",
            &input,
            &json!({"$defs":{"sourceRow":{"type":"string","enum":["r:0:8","r:8:16"]}}}),
        )
        .unwrap()
    }
    #[test]
    fn conflict_decision_wire_roundtrips_typed_references_without_rewriting_hashes_or_text() {
        let initial=simple();let mut input=initial.input.clone();initial.restore(&mut input).unwrap();
        let row=input["source"]["rows"][0][0].clone();
        // Even a hash-shaped slot containing an identity-like string must not
        // be transformed: hashes/IDs of conflicts are opaque canonical bindings.
        input["foundationConflictResolution"]=json!({"manifestSha256":"asset:long:1","causeSha256":"a1"});
        input["foundationConflictManifest"]=json!({"manifestSha256":"asset:long:1","causeSha256":"a1",
            "items":[{"conflictId":"a1","expectedOldHash":"asset:long:1","path":"/assets/asset:long:1/description",
                "sourceAssetId":"asset:long:1","targetAssetId":"asset:long:2",
                "current":{"text":"a1 e1 r1","sourceRefs":[row]},"candidate":{"text":"原文不可删","sourceRefs":[row]}}]});
        input["foundationConflictRepairHistory"]=json!([{"error":"a1 e1 r1","requestSha256":"asset:long:1","rawSha256":"r1"}]);
        let schema=json!({"$defs":{"sourceRow":{"type":"string","enum":[row]}},"properties":{
            "manifestSha256":{"enum":["asset:long:1"]},"decisions":{"items":{"anyOf":[{"properties":{
                "targetAssetId":{"enum":["asset:long:2",null]},"targetVariantId":{"enum":["variant:long:1",null]},
                "expectedOldHash":{"enum":["asset:long:1"]},"conflictId":{"enum":["a1"]},
                "evidenceRows":{"items":{"$ref":"#/$defs/sourceRow"}}}}]}}}});
        let wire=compile("foundation:F2",&input,&schema).unwrap();
        let properties=&wire.schema["properties"]["decisions"]["items"]["anyOf"][0]["properties"];
        assert_eq!(properties["targetAssetId"]["enum"][0],wire.input["registry"][1]["key"]);
        assert_eq!(properties["targetVariantId"]["enum"][0],wire.input["registry"][0]["variants"][0]["key"]);
        assert_eq!(properties["expectedOldHash"]["enum"],schema["properties"]["decisions"]["items"]["anyOf"][0]["properties"]["expectedOldHash"]["enum"]);
        assert_eq!(wire.input["foundationConflictResolution"],input["foundationConflictResolution"]);
        assert_eq!(wire.input["foundationConflictRepairHistory"],input["foundationConflictRepairHistory"]);
        let mut restored=wire.input.clone();wire.restore(&mut restored).unwrap();
        restored["transportInstructions"]=input["transportInstructions"].clone();assert_eq!(restored,input);
        let canonical=json!({"version":"foundation-conflict-resolution/v1","manifestSha256":"asset:long:1","decisions":[{
            "conflictId":"a1","expectedOldHash":"asset:long:1","targetAssetId":"asset:long:2","targetVariantId":"variant:long:1",
            "evidenceRows":[row],"replacementText":"e1 r1 a1","reason":"r1与原文相同"}]});
        let mut response=canonical.clone();wire.walk(&mut response,None,false).unwrap();
        assert_eq!(response["decisions"][0]["evidenceRows"][0],wire.schema["$defs"]["sourceRow"]["enum"][0]);
        wire.restore(&mut response).unwrap();assert_eq!(response,canonical);
        wire.restore(&mut response).unwrap();assert_eq!(response,canonical);
        response["decisions"][0]["evidenceRows"]=json!(["r999999"]);let invalid=response.clone();
        assert!(wire.restore(&mut response).is_err());assert_eq!(response,invalid);
    }
    #[test]
    fn conflict_decision_slots_do_not_change_legacy_compact_compilation() {
        let initial=simple();let mut input=initial.input.clone();initial.restore(&mut input).unwrap();
        input["legacyOpaque"]=json!({"sourceAssetId":"a-new-canonical-id","targetAssetId":"asset:long:2",
            "targetVariantId":"variant:long:1","evidenceRows":["row:0123456789ab:0:8"]});
        let wire=compile("Q1",&input,&json!({})).unwrap();
        assert_eq!(wire.input["legacyOpaque"],input["legacyOpaque"]);
        assert_eq!(wire.input["registry"],initial.input["registry"]);
        let mut response=json!({"targetAssetId":"a2","targetVariantId":"v1","evidenceRows":["r1"]});
        let before=response.clone();wire.restore(&mut response).unwrap();assert_eq!(response,before);
    }
    #[test]
    fn conflict_decision_schema_only_short_canonical_ids_are_reserved_before_aliases() {
        let initial=simple();let mut input=initial.input.clone();initial.restore(&mut input).unwrap();
        input["foundationConflictResolution"]=json!({"manifestSha256":"a1"});
        input["foundationConflictManifest"]=json!({"items":[{"sourceAssetId":"asset:long:1",
            "current":{"id":"a1","name":"a1原文","variants":[{"id":"v1","name":"v1原文"}]}}]});
        let schema=json!({"properties":{"decisions":{"items":{"anyOf":[{"properties":{
            "targetAssetId":{"enum":["a1",null]},"targetVariantId":{"enum":["v1",null]},
            "expectedOldHash":{"enum":["a1"]}}}]}}}});
        let wire=compile("F2",&input,&schema).unwrap();
        assert_ne!(wire.input["registry"][0]["key"],"a1");
        assert_ne!(wire.input["registry"][0]["variants"][0]["key"],"v1");
        let props=&wire.schema["properties"]["decisions"]["items"]["anyOf"][0]["properties"];
        let current=&wire.input["foundationConflictManifest"]["items"][0]["current"];
        assert_eq!(current["id"],props["targetAssetId"]["enum"][0]);
        assert_eq!(current["variants"][0]["id"],props["targetVariantId"]["enum"][0]);
        assert_eq!(current["name"],"a1原文");assert_eq!(props["expectedOldHash"]["enum"][0],"a1");
        let mut response=json!({"decisions":[{"targetAssetId":props["targetAssetId"]["enum"][0],
            "targetVariantId":props["targetVariantId"]["enum"][0],"expectedOldHash":"a1"}]});
        wire.restore(&mut response).unwrap();
        let canonical=json!({"decisions":[{"targetAssetId":"a1","targetVariantId":"v1","expectedOldHash":"a1"}]});
        assert_eq!(response,canonical);wire.restore(&mut response).unwrap();assert_eq!(response,canonical);
        let mut restored=wire.input.clone();wire.restore(&mut restored).unwrap();
        restored["transportInstructions"]=input["transportInstructions"].clone();assert_eq!(restored,input);
        input.as_object_mut().unwrap().remove("foundationConflictResolution");
        let legacy=compile("F2",&input,&schema).unwrap();
        assert_eq!(legacy.input["registry"],initial.input["registry"]);
        assert_eq!(legacy.schema,schema);assert_eq!(legacy.input["foundationConflictManifest"],input["foundationConflictManifest"]);
    }
    #[test]
    fn typed_references_same_names_and_literals_stay_distinct() {
        let wire = simple();
        assert_eq!(wire.input["registry"][0]["name"], "e1");
        assert_eq!(wire.input["registry"][1]["name"], "e1");
        assert_ne!(
            wire.input["registry"][0]["key"],
            wire.input["registry"][1]["key"]
        );
        assert_eq!(wire.input["source"]["rows"][0][5], "e1");
        assert_eq!(wire.input["source"]["rows"][0][9], "e1说r1");
        assert_eq!(
            wire.input["evidence"]["historicalEvidenceCandidates"][0][3],
            "e1 r1 a1"
        );
        assert_eq!(
            wire.input["evidence"]["historicalEvidenceCandidates"][0][4],
            "CHARACTER_CLAIM"
        );
        let mut response = json!({"changes":{"assets":[{"key":wire.input["registry"][0]["key"],"name":"e1","note":"r1说e1","variants":[{"key":wire.input["registry"][0]["variants"][0]["key"],"fromKey":wire.input["registry"][1]["key"],"sceneRefs":[[wire.input["scope"][0],wire.input["scenes"][0][1]]]}]}]},"seriesDirection":{"signals":[{"episodeId":wire.input["scope"][0],"rowId":wire.input["source"]["rows"][0][0],"quote":"e1 r1 a1"}]}});
        wire.restore(&mut response).unwrap();
        assert_eq!(response["changes"]["assets"][0]["key"], "asset:long:1");
        assert_eq!(
            response["changes"]["assets"][0]["variants"][0]["fromKey"],
            "asset:long:2"
        );
        assert_eq!(
            response["changes"]["assets"][0]["variants"][0]["sceneRefs"],
            json!([["episode:long:1", "scene:long:1"]])
        );
        assert_eq!(
            response["seriesDirection"]["signals"][0]["quote"],
            "e1 r1 a1"
        );
        assert_eq!(response["changes"]["assets"][0]["note"], "r1说e1");
    }
    #[test]
    fn applied_decision_subject_assets_roundtrip_with_exact_order_and_duplicates() {
        let initial = simple();
        let mut input = initial.input.clone();
        initial.restore(&mut input).unwrap();
        input["appliedDecisions"] = json!([{
            "id":"AUTHOR:confirmed",
            "subjectAssetKeys":["asset:long:2","asset:long:1","asset:long:1"],
            "previousSubjectAssetKeys":["asset:long:1"],
            "answer":"a1/r1是逐字解释，不是引用槽"
        }]);
        let wire = compile("Q1", &input, &json!({})).unwrap();
        assert_eq!(
            wire.input["appliedDecisions"][0]["subjectAssetKeys"],
            json!([
                wire.input["registry"][1]["key"],
                wire.input["registry"][0]["key"],
                wire.input["registry"][0]["key"]
            ])
        );
        assert_eq!(
            wire.input["appliedDecisions"][0]["previousSubjectAssetKeys"],
            json!([wire.input["registry"][0]["key"]])
        );
        assert_eq!(
            wire.input["appliedDecisions"][0]["answer"],
            input["appliedDecisions"][0]["answer"]
        );
        let mut restored = wire.input.clone();
        wire.restore(&mut restored).unwrap();
        assert_eq!(restored, input);
        // Transport preserves declaration order and duplicates. The domain
        // validator, rather than this alias layer, owns subject normalization.
        wire.restore(&mut restored).unwrap();
        assert_eq!(restored, input);
    }
    #[test]
    fn applied_decision_unknown_subject_aliases_fail_atomically() {
        let wire = simple();
        for field in ["subjectAssetKeys", "previousSubjectAssetKeys"] {
            for invalid in ["a999", "e1", "v1"] {
                let mut response = json!({"appliedDecisions":[{"subjectAssetKeys":[],"previousSubjectAssetKeys":[],"answer":"原解释不改"}]});
                response["appliedDecisions"][0][field] =
                    json!([wire.input["registry"][0]["key"], invalid]);
                let mut copy = response.clone();
                assert!(wire.restore(&mut copy).is_err(), "{field}: {invalid}");
                assert_eq!(
                    copy, response,
                    "rejected aliases must not partially restore prior valid subjects"
                );
            }
        }
    }
    #[test]
    fn exact_source_coordinates_and_closure_coordinates_do_not_leak_task_aliases() {
        let wire = simple();
        let alias = wire.input["source"]["rows"][0][0].clone();
        let mut output = json!({"issues":[{"coordinate":alias,"problem":"r1是原样说明","sourceRefs":[alias]}],"resolvedIssues":[alias,"asset-review-custom"],"humanChoices":[{"decisionKey":"r1_custom_choice","question":"r1是原样问题"}]});
        wire.restore(&mut output).unwrap();
        assert_eq!(output["issues"][0]["coordinate"], "row:0123456789ab:0:8");
        assert_eq!(output["resolvedIssues"][0], "row:0123456789ab:0:8");
        assert_eq!(output["resolvedIssues"][1], "asset-review-custom");
        assert_eq!(output["humanChoices"][0]["decisionKey"], "r1_custom_choice");
        assert_eq!(output["issues"][0]["problem"], "r1是原样说明");
        assert_eq!(output["humanChoices"][0]["question"], "r1是原样问题");
        let canonical = output.clone();
        wire.restore(&mut output).unwrap();
        assert_eq!(output, canonical);
    }
    #[test]
    fn unknown_short_references_and_type_confusion_fail_atomically() {
        let wire = simple();
        for response in [
            json!({"sourceRefs":["r999"]}),
            json!({"episodeId":"s1"}),
            json!({"episodeId":"scene:long:1"}),
            json!({"fromKey":"e1"}),
            json!({"changes":{"assets":[{"key":"a999"}]}}),
            json!({"sceneId":"s999__L164"}),
        ] {
            let mut copy = response.clone();
            assert!(wire.restore(&mut copy).is_err());
            assert_eq!(copy, response);
        }
        let mut split =
            json!({"sceneId":format!("{}__L164",wire.input["scenes"][0][1].as_str().unwrap())});
        wire.restore(&mut split).unwrap();
        assert_eq!(split["sceneId"], "scene:long:1__L164");
        wire.restore(&mut split).unwrap();
        assert_eq!(split["sceneId"], "scene:long:1__L164");
    }
    #[test]
    fn dictionary_history_preserves_rows_claim_and_links() {
        let mut input = simple().input;
        // Start from canonical rather than using an already compiled request.
        let first = simple();
        first.restore(&mut input).unwrap();
        input["evidence"]["historicalEvidenceCandidates"] = json!({"columns":["rowId","episodeId","sceneId","text","claim","retrievalMode","anchorRowIds"],"dictionaries":{"episodeId":["episode:history"],"sceneId":["scene:history"],"text":["e1 r1 a1"],"claim":["CHARACTER_CLAIM"]},"rows":[["row:0123456789ab:8:16",0,0,0,0,"SUBJECT_MENTION",[0]]]});
        let wire = compile("Q1", &input, &json!({})).unwrap();
        assert_eq!(
            wire.input["evidence"]["historicalEvidenceCandidates"]["dictionaries"]["claim"][0],
            "CHARACTER_CLAIM"
        );
        assert_eq!(
            wire.input["evidence"]["historicalEvidenceCandidates"]["dictionaries"]["text"][0],
            "e1 r1 a1"
        );
        assert_eq!(
            wire.input["evidence"]["historicalEvidenceCandidates"]["rows"][0][6],
            json!([0])
        );
        let mut restored = wire.input.clone();
        wire.restore(&mut restored).unwrap();
        assert_eq!(restored, input);
    }
    #[test]
    fn alias_like_canonical_ids_do_not_collide_and_profile_is_closed() {
        let input = json!({"modelInputProfile":PROFILE,"scope":["e1","episode:long"],"registry":[{"key":"a1","name":"a1"},{"key":"asset:long","name":"a1"}]});
        let wire = compile("F2", &input, &json!({})).unwrap();
        assert_ne!(wire.input["scope"][0], "e1");
        let mut restored = wire.input.clone();
        wire.restore(&mut restored).unwrap();
        restored
            .as_object_mut()
            .unwrap()
            .remove("transportInstructions");
        assert_eq!(restored, input);
        assert!(compile("storyboard:plan", &input, &json!({})).is_err());
        assert!(compile("F2", &json!({"modelInputProfile":"unknown"}), &json!({})).is_err());
        assert!(compile("F2", &json!({}), &json!({})).is_err());
    }
    #[test]
    fn material_candidates_and_batch_external_identities_are_typed() {
        let mut input = simple().input;
        let initial = simple();
        initial.restore(&mut input).unwrap();
        input["candidates"] = json!({"assets":[],"materialCandidates":[{"kind":"BASE","key":"asset:unlocated","baseKey":"asset:unlocated","name":"e1","episodes":[],"sceneRefs":[]},{"kind":"VARIANT","key":"variant:unlocated","baseKey":"asset:unlocated","name":"r1","episodes":[],"sceneRefs":[]}],"crossBatchVariants":[{"key":"variant:external","name":"s1","fromKey":"variant:unlocated","preparation":{"mode":"DERIVE","fromKey":"asset:unlocated"}}]});
        let wire = compile("Q1", &input, &json!({})).unwrap();
        assert!(wire.input["candidates"]["materialCandidates"][0]["key"]
            .as_str()
            .unwrap()
            .starts_with('a'));
        assert!(wire.input["candidates"]["materialCandidates"][1]["key"]
            .as_str()
            .unwrap()
            .starts_with('v'));
        assert!(wire.input["candidates"]["crossBatchVariants"][0]["key"]
            .as_str()
            .unwrap()
            .starts_with('v'));
        assert!(
            wire.input["candidates"]["crossBatchVariants"][0]["preparation"]["fromKey"]
                .as_str()
                .unwrap()
                .starts_with('a')
        );
        let mut restored = wire.input.clone();
        wire.restore(&mut restored).unwrap();
        assert_eq!(restored, input);
        let mut bad = input.clone();
        bad["registry"][1]["key"] = bad["registry"][0]["variants"][0]["key"].clone();
        assert!(compile("Q1", &bad, &json!({})).is_err());
        assert!(compile(
            "Q1",
            &input,
            &json!({"$defs":{"sourceRow":{"enum":["r:0:999999"]}}})
        )
        .is_err());
    }
}
