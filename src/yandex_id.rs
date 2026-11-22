//! Иконки меню профиля. В оригинале это меню — iframe Яндекс ID (yandex.ru/user-id), и его
//! иконки приходят с сайта Яндекс ID, а не из app.asar. Поэтому при первом запуске код этой
//! страницы скачивается (около 1,5 МБ, один раз), иконки находятся в нём по началу контура
//! и сохраняются готовыми SVG в %APPDATA%\yamusic-rs\assets\id. Если сайт поменяется так,
//! что иконки не найдутся, меню просто покажется без них.

use crate::config::log;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const PAGE: &str = "https://yandex.ru/user-id";

/// Имя файла и начало первого контура иконки в коде Яндекс ID.
const ICONS: [(&str, &str); 10] = [
    ("check", "M19.716 4.386"),
    ("close", "M19.207 6.207"),
    ("id", "M17.129 9.227"),
    ("settings", "M9.172 9.172"),
    ("support", "M5.719 2.535"),
    ("language", "M22 11.998"),
    ("appearance", "M15.106.409"),
    ("dark", "M3.57 17.898"),
    ("system", "M3 7a3 3 0 0 1 3-3h12"),
    ("logo", "M17.2116 8.40037"),
];

/// Готовые SVG по именам: значки (одноцветные, цвет задаёт `colorize` в Slint),
/// `logo` и `logo-dark` — логотип «Я ID» для светлой и тёмной темы.
pub type Svgs = HashMap<String, String>;

fn dir() -> PathBuf {
    crate::config::cache_dir().join("assets").join("id")
}

fn names() -> impl Iterator<Item = String> {
    ICONS.iter().filter(|(n, _)| *n != "logo").map(|(n, _)| n.to_string()).chain(["logo".into(), "logo-dark".into()])
}

/// Иконки из кэша. `None`, если хотя бы одной нет — тогда нужен `download`.
pub fn cached() -> Option<Svgs> {
    names().map(|n| std::fs::read_to_string(dir().join(format!("{n}.svg"))).ok().map(|s| (n, s))).collect()
}

/// Скачать код Яндекс ID и собрать иконки. Долго: звать из фонового потока.
pub fn download() -> Result<Svgs, String> {
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let get = |url: &str| -> Result<String, String> {
        let r = http.get(url).send().map_err(|e| format!("{url}: {}", crate::config::chain(&e)))?;
        if !r.status().is_success() {
            return Err(format!("{url}: HTTP {}", r.status()));
        }
        r.text().map_err(|e| format!("{url}: {}", crate::config::chain(&e)))
    };

    let page = get(PAGE)?;
    let assets = between(&page, "__ASSETS_URL__ = \"", "\"").ok_or("на странице нет адреса ресурсов")?;
    let app_url = attr_ending(&page, "src=\"", "/app.js").ok_or("на странице нет app.js")?;
    let css_url = attr_ending(&page, "href=\"", ".css").ok_or("на странице нет стилей")?;
    let app = get(&app_url)?;
    let css = get(&css_url)?;

    // Иконки лежат и в самом app.js, и в мелких чанках по одной. Чанки качаем, пока не найдём все.
    let mut found: HashMap<&str, String> = HashMap::new();
    let scan = |src: &str, found: &mut HashMap<&str, String>| {
        for (name, mark) in ICONS {
            if !found.contains_key(name) {
                if let Some(el) = svg_around(src, mark) {
                    found.insert(name, el);
                }
            }
        }
    };
    scan(&app, &mut found);
    let mut chunks: Vec<&str> = Vec::new();
    for (i, _) in app.match_indices("chunks/chunk-") {
        let name = &app[i + "chunks/".len()..];
        let Some(end) = name.find(".js") else { continue };
        let name = &name[..end + 3];
        if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') && !chunks.contains(&name) {
            chunks.push(name);
        }
    }
    for chunk in chunks {
        if found.len() == ICONS.len() {
            break;
        }
        match get(&format!("{assets}static/chunks/{chunk}")) {
            Ok(src) => scan(&src, &mut found),
            Err(e) => log(format!("Яндекс ID: {e}")),
        }
    }

    let vars = css_vars(&css);
    let mut out = Svgs::new();
    for (name, _) in ICONS {
        let Some(el) = found.get(name) else {
            log(format!("Яндекс ID: иконка {name} не найдена"));
            continue;
        };
        if name == "logo" {
            for (file, scheme) in [("logo", "light"), ("logo-dark", "dark")] {
                if let Some(svg) = to_svg(el, &logo_vars(&css, &vars, scheme)) {
                    out.insert(file.into(), svg);
                }
            }
        } else if let Some(svg) = to_svg(el, &HashMap::new()) {
            out.insert(name.into(), svg);
        }
    }
    Ok(out)
}

pub fn save(svgs: &Svgs) {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    for (name, svg) in svgs {
        let _ = std::fs::write(dir.join(format!("{name}.svg")), svg);
    }
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    Some(&s[a..a + s[a..].find(end)?])
}

/// Значение атрибута `prefix…`, которое заканчивается на `suffix` (адрес app.js, адрес css).
fn attr_ending(s: &str, prefix: &str, suffix: &str) -> Option<String> {
    s.match_indices(prefix).find_map(|(i, _)| {
        let v = &s[i + prefix.len()..];
        let v = &v[..v.find('"')?];
        (v.starts_with("https://") && v.ends_with(suffix)).then(|| v.to_string())
    })
}

/// Вызов `jsx("svg", …)` (React после сборки), внутри которого есть контур `mark`.
/// Кусок кончается там, где начинается следующая иконка.
fn svg_around(src: &str, mark: &str) -> Option<String> {
    let at = src.find(&format!("d:\"{mark}"))?;
    let start = src[..at].rfind("(\"svg\",")?;
    let end = src[at..].find("(\"svg\",").map_or(src.len(), |e| at + e);
    Some(src[start..end].to_string())
}

/// Собрать SVG из вызовов `jsx("path", {…})`. Ветку `else` у `язык === "ru" ? … : …` пропускаем:
/// она перед вызовом стоит после `):`, а у дочернего элемента перед ним `children:`.
fn to_svg(el: &str, vars: &HashMap<String, String>) -> Option<String> {
    let view_box = between(el, "viewBox:\"", "\"")?;
    let root_fill = if el[..el.find("children")?].contains("fill:\"none\"") { " fill=\"none\"" } else { "" };
    let mut body = String::new();
    let mut rest = el;
    while let Some(i) = rest.find(".jsx(\"") {
        let before = rest[..i].trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        let call = &rest[i + ".jsx(\"".len()..];
        rest = call;
        let Some(tag_end) = call.find('"') else { break };
        let tag = &call[..tag_end];
        if !matches!(tag, "path" | "circle" | "rect") {
            continue;
        }
        if before.ends_with("):") {
            continue;
        }
        let props = between(call, "{", "}")?;
        body.push('<');
        body.push_str(tag);
        for (key, value) in props_of(props) {
            let value = resolve(&value, vars);
            let value = if value == "currentColor" { "#000".to_string() } else { value };
            body.push_str(&format!(" {}=\"{}\"", kebab(key), value.replace('"', "")));
        }
        body.push_str("/>");
    }
    if body.is_empty() {
        return None;
    }
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{view_box}\"{root_fill}>{body}</svg>");
    // Проверка, что Slint такое прочитает.
    slint::Image::load_from_svg_data(svg.as_bytes()).ok()?;
    Some(svg)
}

/// Пары `ключ:"значение"` из объекта свойств.
fn props_of(props: &str) -> Vec<(&str, String)> {
    let mut out = Vec::new();
    let mut rest = props;
    while let Some(colon) = rest.find(":\"") {
        let key = rest[..colon].rsplit([',', '{']).next().unwrap_or("").trim();
        let value_start = colon + 2;
        let Some(len) = rest[value_start..].find('"') else { break };
        if key.chars().all(|c| c.is_ascii_alphabetic()) && !key.is_empty() {
            out.push((key, rest[value_start..value_start + len].to_string()));
        }
        rest = &rest[value_start + len + 1..];
    }
    out
}

fn kebab(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Все объявления `--имя: значение` из CSS (первое встреченное).
fn css_vars(css: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for (i, _) in css.match_indices("--") {
        let decl = &css[i..];
        let Some(colon) = decl.find(':') else { continue };
        let name = &decl[..colon];
        if !name[2..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || name.len() < 3 {
            continue;
        }
        let value = &decl[colon + 1..];
        let Some(end) = value.find([';', '}']) else { continue };
        vars.entry(name.to_string()).or_insert_with(|| value[..end].trim().to_string());
    }
    vars
}

/// Цвета логотипа: правило с `--id-logo-short-width` (знак «Я ID» без надписи «Яндекс»)
/// и основной цвет текста нужной темы.
fn logo_vars(css: &str, all: &HashMap<String, String>, scheme: &str) -> HashMap<String, String> {
    let mut vars = all.clone();
    if let Some(at) = css.find("var(--id-logo-short-width)") {
        let start = css[..at].rfind('{').map_or(0, |s| s + 1);
        let end = at + css[at..].find('}').unwrap_or(0);
        vars.extend(css_vars(&format!("{}}}", &css[start..end])));
    }
    vars.insert("--id-color-text-primary".into(), format!("var(--id-default-color-{scheme}-text-primary)"));
    vars
}

/// Раскрыть `var(--имя)` по таблице, с защитой от зацикливания.
fn resolve(value: &str, vars: &HashMap<String, String>) -> String {
    let mut v = value.trim().to_string();
    for _ in 0..8 {
        let Some(name) = v.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) else { break };
        let name = name.split(',').next().unwrap_or("").trim();
        match vars.get(name) {
            Some(next) => v = next.trim().to_string(),
            None => break,
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_to_svg() {
        let el = r#"("svg",e(s({width:24,fill:"none",viewBox:"0 0 24 24"},r),{children:a.jsx("path",{fill:"currentColor",fillRule:"evenodd",d:"M1 1h2v2z"})}))"#;
        assert_eq!(
            to_svg(el, &HashMap::new()).unwrap(),
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"><path fill="#000" fill-rule="evenodd" d="M1 1h2v2z"/></svg>"##
        );
    }

    #[test]
    fn ternary_else_skipped() {
        let el = r#"("svg",{viewBox:"0 0 62 30",children:[s.jsx("circle",{cx:"15",r:"15",fill:"var(--bg)"}),"ru"===o?s.jsx("path",{d:"M1 1z",fill:"red"}):s.jsx("path",{d:"M2 2z",fill:"red"})]})"#;
        let vars = HashMap::from([("--bg".to_string(), "#fc3f1d".to_string())]);
        let svg = to_svg(el, &vars).unwrap();
        assert!(svg.contains(r##"<circle cx="15" r="15" fill="#fc3f1d"/>"##));
        assert!(svg.contains("M1 1z") && !svg.contains("M2 2z"));
    }

    /// Нужна сеть: проверяет, что на живом сайте находятся все иконки.
    #[test]
    #[ignore]
    fn live() {
        let svgs = download().unwrap();
        assert_eq!(svgs.len(), 11, "{:?}", svgs.keys());
    }
}
