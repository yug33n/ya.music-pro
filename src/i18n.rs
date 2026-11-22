//! Перевод интерфейса словарями официального приложения (ru, en, uz, kk) из app.asar.
//! Словарь — «русская строка → строка на выбранном языке»: русские строки интерфейса
//! совпадают с русским словарём оригинала, так что ключи не нужны.

use crate::assets::Asar;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

const CHUNKS: &str = "app/_next/static/chunks";

thread_local! {
    static DICT: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Строка на текущем языке; если перевода нет — как есть.
pub fn tr(s: &str) -> String {
    DICT.with(|d| d.borrow().get(s).cloned()).unwrap_or_else(|| s.to_string())
}

/// Загружает словарь для `lang`. Для ru словарь пустой.
pub fn set_lang(lang: &str) {
    let dict = if lang == "ru" { HashMap::new() } else { load(lang).unwrap_or_default() };
    DICT.with(|d| *d.borrow_mut() = dict);
}

/// Есть ли откуда взять словари: без официального клиента остаётся только русский.
pub fn available() -> bool {
    Asar::open().is_some()
}

fn load(lang: &str) -> Option<HashMap<String, String>> {
    let mut asar = Asar::open()?;
    let mut ru = None;
    let mut target = None;
    for name in asar.list(CHUNKS) {
        if !name.ends_with(".js") {
            continue;
        }
        let path = format!("{CHUNKS}/{name}");
        let head = asar.read_prefix(&path, 200)?;
        if !String::from_utf8_lossy(&head).contains("JSON.parse('{\"a11y") {
            continue;
        }
        let Some(dict) = asar.read(&path).and_then(|b| parse(&String::from_utf8_lossy(&b))) else { continue };
        match detect(&dict) {
            "ru" => ru = Some(dict),
            l if l == lang => target = Some(dict),
            _ => {}
        }
    }
    let (ru, target) = (ru?, target?);
    let mut out = HashMap::new();
    for (key, text) in ru {
        if let Some(t) = target.get(&key) {
            out.entry(text).or_insert_with(|| t.clone());
        }
    }
    Some(out)
}

/// Язык словаря по буквам: казахские — kk, латиница с oʻ/gʻ — uz, латиница — en, иначе ru.
fn detect(d: &HashMap<String, String>) -> &'static str {
    let sample: String = ["a11y-regions.player", "entity-names.track", "entity-names.top-artists", "entity-names.text"]
        .iter()
        .filter_map(|k| d.get(*k))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let all: String = d.values().take(300).cloned().collect();
    if all.chars().any(|c| "қңүұәөғһі".contains(c)) {
        "kk"
    } else if sample.chars().any(|c| c.is_ascii_alphabetic()) {
        if all.contains('ʻ') || all.contains('‘') { "uz" } else { "en" }
    } else {
        "ru"
    }
}

/// `JSON.parse('…')` webpack-чанка → ключ → текст. Сообщения с выбором (числа, пол) пропускаем,
/// аргументы оставляем как `{name}`.
fn parse(js: &str) -> Option<HashMap<String, String>> {
    let a = js.find("JSON.parse('")? + "JSON.parse('".len();
    let b = js.rfind("')")?;
    let v: Value = serde_json::from_str(&unescape_js(js.get(a..b)?)).ok()?;
    let mut out = HashMap::new();
    for (k, parts) in v.as_object()? {
        let Some(parts) = parts.as_array() else { continue };
        let mut s = String::new();
        let mut ok = true;
        for p in parts {
            let value = p.get("value").and_then(|x| x.as_str()).unwrap_or("");
            match p.get("type").and_then(|x| x.as_u64()) {
                Some(0) => s.push_str(value),
                Some(1) => {
                    s.push('{');
                    s.push_str(value);
                    s.push('}');
                }
                _ => ok = false,
            }
        }
        if ok {
            // Неразрывные пробелы — обычными: так они и рисуются у нас (см. group_digits).
            out.insert(k.clone(), s.replace('\u{a0}', " "));
        }
    }
    Some(out)
}

fn unescape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                out.extend(u32::from_str_radix(&h, 16).ok().and_then(char::from_u32));
            }
            Some('u') => {
                let h: String = it.by_ref().take(4).collect();
                out.extend(u32::from_str_radix(&h, 16).ok().and_then(char::from_u32));
            }
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}
