//! Independent visible v7 reader. It does not call the renderer or use an expected object.
use super::*;
use regex::Regex;
struct Reader<'a> {
    lines: Vec<&'a str>,
    pos: usize,
}
impl<'a> Reader<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.pos).copied()
    }
    fn take(&mut self) -> Result<&'a str> {
        let v = self.peek().ok_or("VERIFY_SYNTAX: unexpected end")?;
        self.pos += 1;
        Ok(v)
    }
    fn expect(&mut self, v: &str) -> Result<()> {
        need(self.take()? == v, "VERIFY_SYNTAX", v)
    }
    fn scene_start(&self) -> bool {
        self.peek().is_some()
            && ["场景：", "人物：", "道具："]
                .iter()
                .enumerate()
                .all(|(i, p)| {
                    self.lines
                        .get(self.pos + i + 1)
                        .is_some_and(|s| s.starts_with(p))
                })
    }
    fn continued(&mut self, first: &str) -> String {
        let mut out = first.to_string();
        while self.peek().is_some_and(|s| s.starts_with("  ")) && !self.scene_start() {
            out.push('\n');
            out.push_str(&self.lines[self.pos][2..]);
            self.pos += 1;
        }
        out
    }
    fn tag(&mut self) -> Result<Value> {
        let text = self.take()?;
        let rest = text
            .strip_prefix('[')
            .ok_or("VERIFY_SYNTAX: expected tag")?;
        let (label, body) = rest.split_once("] ").ok_or("VERIFY_SYNTAX: invalid tag")?;
        need(
            !label.is_empty() && !label.contains(']'),
            "VERIFY_SYNTAX",
            "bad tag label",
        )?;
        Ok(json!([label, self.continued(body)]))
    }
    fn field(&mut self, label: &str) -> Result<Value> {
        let text = self.take()?;
        let body = text
            .strip_prefix(&format!("{label}："))
            .ok_or_else(|| format!("VERIFY_SYNTAX: expected {label}"))?;
        Ok(json!([label, self.continued(body)]))
    }
}
fn decimal_ticks(s: &str) -> Result<i64> {
    let (a, b) = s.split_once('.').unwrap_or((s, ""));
    need(
        !a.is_empty()
            && a.bytes().all(|b| b.is_ascii_digit())
            && b.len() <= 9
            && b.bytes().all(|b| b.is_ascii_digit()),
        "VERIFY_SYNTAX",
        "invalid decimal",
    )?;
    let whole: i64 = a.parse().map_err(|_| "VERIFY_SYNTAX: time overflow")?;
    let fraction: i64 = if b.is_empty() {
        0
    } else {
        format!("{b:0<9}")
            .parse()
            .map_err(|_| "VERIFY_SYNTAX: time fraction")?
    };
    whole
        .checked_mul(TICKS)
        .and_then(|n| n.checked_add(fraction))
        .ok_or_else(|| "VERIFY_SYNTAX: time overflow".into())
}
pub(super) fn parse(markdown: &str) -> Result<Value> {
    need(
        markdown.ends_with('\n') && !markdown.contains('\r'),
        "VERIFY_SYNTAX",
        "LF with final newline required",
    )?;
    let mut r = Reader {
        lines: markdown[..markdown.len() - 1].split('\n').collect(),
        pos: 0,
    };
    r.expect("**【标准化剧本】**")?;
    let episode_re = Regex::new(r"^第(-?\d+)集$").unwrap();
    let fragment_re = Regex::new(r"^片段(\d+)$").unwrap();
    let duration_re = Regex::new(r"^时长：(\d+)秒$").unwrap();
    let shot_re =
        Regex::new(r"^分镜(\d+)｜(\d+(?:\.\d{1,9})?)-(\d+(?:\.\d{1,9})?)s｜(\d+(?:\.\d{1,9})?)s$")
            .unwrap();
    let before_re =
        Regex::new(r"^\[(分段标题|分段演出|光影承接|人物状态|分段结束状态)｜分镜\d+—\d+\] ")
            .unwrap();
    let mut episodes = vec![];
    let mut total_scenes = 0;
    while r.peek().is_some() {
        let line = r.take()?;
        let cap = episode_re
            .captures(line)
            .ok_or("VERIFY_SYNTAX: episode heading missing")?;
        let number: i64 = cap[1]
            .parse()
            .map_err(|_| "VERIFY_SYNTAX: episode number overflow")?;
        let mut scenes = vec![];
        while r.peek().is_some() && (r.scene_start() || !episode_re.is_match(r.peek().unwrap())) {
            need(
                r.scene_start(),
                "VERIFY_SYNTAX",
                "scene must be followed by three asset rows",
            )?;
            let id = r.take()?.to_string();
            let mut assets = vec![];
            for label in ["场景", "人物", "道具"] {
                assets.push(r.field(label)?[1].clone());
            }
            let mut units = vec![];
            while !r.scene_start() && r.peek().is_some_and(|s| fragment_re.is_match(s)) {
                let line = r.take()?;
                let cap = fragment_re.captures(line).unwrap();
                let unit_no: u64 = cap[1]
                    .parse()
                    .map_err(|_| "VERIFY_SYNTAX: fragment number overflow")?;
                let line = r.take()?;
                let cap = duration_re
                    .captures(line)
                    .ok_or("VERIFY_SYNTAX: container duration missing")?;
                let duration: i64 = cap[1]
                    .parse()
                    .map_err(|_| "VERIFY_SYNTAX: duration overflow")?;
                need(
                    (4..=30).contains(&duration),
                    "VERIFY_SYNTAX",
                    "container must be 4–30 seconds",
                )?;
                let mut header = vec![];
                for label in [
                    "通用提示词",
                    "场景",
                    "本单元道具",
                    "本单元出场人物",
                    "本单元光影",
                    "人物状态与站位前提",
                ] {
                    let f = r.tag()?;
                    need(
                        f[0] == label,
                        "VERIFY_SYNTAX",
                        "header label/order mismatch",
                    )?;
                    header.push(f);
                }
                while r.peek().is_some_and(|s| {
                    s.starts_with("[单元光影] ") || s.starts_with("[单元起始状态] ")
                }) {
                    header.push(r.tag()?);
                }
                let mut previous = 0i64;
                let mut shots = vec![];
                while r.peek().is_some()
                    && !r.scene_start()
                    && !r.peek().unwrap().starts_with("[结束状态] ")
                {
                    let mut before = vec![];
                    while r.peek().is_some_and(|s| before_re.is_match(s)) {
                        before.push(r.tag()?);
                    }
                    let line = r.take()?;
                    let cap = shot_re
                        .captures(line)
                        .ok_or("VERIFY_SYNTAX: shot time line missing")?;
                    let number: u64 = cap[1]
                        .parse()
                        .map_err(|_| "VERIFY_SYNTAX: shot number overflow")?;
                    let start = decimal_ticks(&cap[2])?;
                    let end = decimal_ticks(&cap[3])?;
                    let duration = decimal_ticks(&cap[4])?;
                    need(
                        start == previous
                            && duration > 0
                            && end.checked_sub(start) == Some(duration),
                        "VERIFY_SYNTAX",
                        "shot continuity or duration mismatch",
                    )?;
                    previous = end;
                    let mut fields = vec![];
                    for label in ["机位/运镜", "画面", "光影", "声音"] {
                        fields.push(r.field(label)?);
                    }
                    fields.push(r.field("台词")?);
                    while r.peek().is_some_and(|s| s.starts_with("台词：")) {
                        fields.push(r.field("台词")?);
                    }
                    fields.push(r.field("转场")?);
                    shots.push(json!({"number":number,"start":start as f64/TICKS as f64,"end":end as f64/TICKS as f64,"duration":duration as f64/TICKS as f64,"before":before,"fields":fields}));
                }
                need(!shots.is_empty(), "VERIFY_SYNTAX", "fragment lacks shots")?;
                let tail = r.tag()?;
                need(
                    tail[0] == "结束状态",
                    "VERIFY_SYNTAX",
                    "fragment lacks exit state",
                )?;
                need(
                    previous <= 30 * TICKS && container_seconds(previous) == duration,
                    "VERIFY_SYNTAX",
                    "container must round actual duration upward",
                )?;
                units.push(json!({"number":unit_no,"duration":duration,"header":header,"shots":shots,"footer":[tail]}));
            }
            need(!units.is_empty(), "VERIFY_SYNTAX", "scene lacks fragments")?;
            total_scenes += 1;
            scenes.push(json!({"id":id,"assets":assets,"units":units}));
        }
        need(!scenes.is_empty(), "VERIFY_SYNTAX", "episode lacks scenes")?;
        episodes.push(json!({"number":number,"scenes":scenes}));
    }
    need(
        total_scenes > 0,
        "LINGMO_SCENE_PROP_BUDGET",
        "no scene asset triplet",
    )?;
    Ok(json!({"profile":"lingmo-v7","episodes":episodes}))
}
