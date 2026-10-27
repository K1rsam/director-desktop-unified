//! Frozen Unicode 17.0 tables: Python isalnum/NFKD/NFC/casefold/lower and
//! Node Unicode Cased/Case_Ignorable properties. No subprocess at runtime.
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Tables {
    alnum: Vec<[u32; 2]>,
    cased: Vec<[u32; 2]>,
    ignorable: Vec<[u32; 2]>,
    nfkd: HashMap<String, String>,
    ccc: HashMap<String, u8>,
    compose: HashMap<String, String>,
    fold: HashMap<String, String>,
    lower: HashMap<String, String>,
}
fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        serde_json::from_str(include_str!("series_lite_registry_unicode17.json"))
            .expect("frozen Unicode table")
    })
}
fn contains(ranges: &[[u32; 2]], c: char) -> bool {
    let n = c as u32;
    let i = ranges.partition_point(|r| r[1] < n);
    ranges.get(i).is_some_and(|r| r[0] <= n)
}
pub fn python_alnum(c: char) -> bool {
    contains(&tables().alnum, c)
}
fn ccc(c: char) -> u8 {
    *tables().ccc.get(&(c as u32).to_string()).unwrap_or(&0)
}
fn compose(a: char, b: char) -> Option<char> {
    let (a, b) = (a as u32, b as u32);
    if (0x1100..0x1113).contains(&a) && (0x1161..0x1176).contains(&b) {
        return char::from_u32(0xac00 + (a - 0x1100) * 588 + (b - 0x1161) * 28);
    }
    if (0xac00..0xd7a4).contains(&a) && (a - 0xac00) % 28 == 0 && (0x11a8..0x11c3).contains(&b) {
        return char::from_u32(a + b - 0x11a7);
    }
    tables()
        .compose
        .get(&format!("{a},{b}"))
        .and_then(|s| s.chars().next())
}
pub fn nfkc(text: &str) -> String {
    let mut decomposed = Vec::new();
    for c in text.chars() {
        let n = c as u32;
        if (0xac00..0xd7a4).contains(&n) {
            let i = n - 0xac00;
            decomposed.push(char::from_u32(0x1100 + i / 588).unwrap());
            decomposed.push(char::from_u32(0x1161 + (i % 588) / 28).unwrap());
            if i % 28 != 0 {
                decomposed.push(char::from_u32(0x11a7 + i % 28).unwrap());
            }
        } else if let Some(s) = tables().nfkd.get(&n.to_string()) {
            decomposed.extend(s.chars());
        } else {
            decomposed.push(c);
        }
    }
    // Stable canonical ordering, bounded to each starter segment.
    for i in 1..decomposed.len() {
        let class = ccc(decomposed[i]);
        if class == 0 {
            continue;
        }
        let mut j = i;
        while j > 0 && ccc(decomposed[j - 1]) > class {
            decomposed.swap(j, j - 1);
            j -= 1;
        }
    }
    let mut out = Vec::new();
    let mut starter = None;
    let mut previous = 0;
    for c in decomposed {
        let class = ccc(c);
        if let Some(i) = starter {
            if previous == 0 || previous < class {
                if let Some(composite) = compose(out[i], c) {
                    out[i] = composite;
                    continue;
                }
            }
        }
        if class == 0 {
            starter = Some(out.len());
        }
        previous = class;
        out.push(c);
    }
    out.into_iter().collect()
}
pub fn casefold(text: &str) -> String {
    text.chars()
        .map(|c| {
            tables()
                .fold
                .get(&(c as u32).to_string())
                .cloned()
                .unwrap_or_else(|| c.to_string())
        })
        .collect()
}
pub fn lowercase_en(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().copied().enumerate() {
        if c == 'Σ' {
            let before = chars[..i]
                .iter()
                .rev()
                .find(|c| !contains(&tables().ignorable, **c))
                .is_some_and(|c| contains(&tables().cased, *c));
            let after = chars[i + 1..]
                .iter()
                .find(|c| !contains(&tables().ignorable, **c))
                .is_some_and(|c| contains(&tables().cased, *c));
            if before && !after {
                out.push('ς');
                continue;
            }
        }
        if let Some(s) = tables().lower.get(&(c as u32).to_string()) {
            out.push_str(s);
        } else {
            out.push(c);
        }
    }
    out
}
