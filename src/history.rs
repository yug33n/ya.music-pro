//! История прослушиваний: дни, в каждом — откуда слушали и какие треки.

use crate::api::{hex_color, parse_album, parse_artist, parse_playlist, parse_track, str_of, Api, Res, Track};
use crate::landing::MONTHS;
use chrono::{Datelike, NaiveDate};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub struct Day {
    /// "2026-11-21"
    pub date: String,
    pub groups: Vec<Group>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Wave,
    Album,
    Playlist,
    Artist,
    Other,
}

pub struct Group {
    pub kind: Kind,
    /// `album:<id>`, `playlist:<uid>:<kind>`, `artist:<id>`, у волны — станция.
    pub target: String,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<String>,
    pub bg: Option<[u8; 3]>,
    pub tracks: Vec<Track>,
}

/// Больше оригинал не показывает.
const MAX_TRACKS: usize = 1000;

pub fn load(api: &Api) -> Res<Vec<Day>> {
    let mut v = api.get_raw("/music-history?fullModelsCount=50")?;
    trim(&mut v);
    fill(api, &mut v);
    Ok(parse(&v))
}

fn arr<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(|a| a.as_array()).into_iter().flatten()
}

fn arr_mut<'a>(v: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
    v.get_mut(key).and_then(|a| a.as_array_mut()).into_iter().flatten()
}

fn trim(v: &mut Value) {
    let mut left = MAX_TRACKS;
    for tab in arr_mut(v, "historyTabs") {
        for it in arr_mut(tab, "items") {
            if let Some(t) = it.get_mut("tracks").and_then(|t| t.as_array_mut()) {
                t.truncate(left);
                left -= t.len();
            }
        }
    }
}

/// Контексты и треки дня по очереди.
fn each(v: &mut Value, f: &mut impl FnMut(&mut Value)) {
    for tab in arr_mut(v, "historyTabs") {
        for it in arr_mut(tab, "items") {
            if let Some(c) = it.get_mut("context") {
                f(c);
            }
            for t in arr_mut(it, "tracks") {
                f(t);
            }
        }
    }
}

fn key(it: &Value) -> String {
    format!("{}{}", str_of(it, "type"), it.pointer("/data/itemId").map(|x| x.to_string()).unwrap_or_default())
}

/// Полные данные сервер присылает только для первых треков, остальное — запросом items.
fn fill(api: &Api, v: &mut Value) {
    let mut seen = HashSet::new();
    let mut want = Vec::new();
    each(v, &mut |it| {
        if it.pointer("/data/fullModel").is_none() && seen.insert(key(it)) {
            want.push(json!({ "type": it.get("type"), "data": { "itemId": it.pointer("/data/itemId") } }));
        }
    });
    let mut got = HashMap::new();
    for chunk in want.chunks(200) {
        match api.post_raw("/music-history/items", &json!({ "items": chunk })) {
            Ok(r) => {
                for it in arr(&r, "items") {
                    if let Some(fm) = it.pointer("/data/fullModel") {
                        got.insert(key(it), fm.clone());
                    }
                }
            }
            Err(e) => {
                crate::config::log(format!("история: {e}"));
                break;
            }
        }
    }
    each(v, &mut |it| {
        if it.pointer("/data/fullModel").is_none() {
            if let (Some(fm), Some(data)) = (got.get(&key(it)), it.get_mut("data").and_then(|d| d.as_object_mut())) {
                data.insert("fullModel".into(), fm.clone());
            }
        }
    });
}

fn parse(v: &Value) -> Vec<Day> {
    arr(v, "historyTabs")
        .map(|tab| Day { date: str_of(tab, "date"), groups: arr(tab, "items").map(group).filter(|g| !g.tracks.is_empty()).collect() })
        .filter(|d| !d.groups.is_empty())
        .collect()
}

fn group(it: &Value) -> Group {
    let c = it.get("context").unwrap_or(&Value::Null);
    let fm = c.pointer("/data/fullModel").unwrap_or(&Value::Null);
    let mut g = Group {
        kind: Kind::Other,
        target: String::new(),
        title: String::new(),
        subtitle: String::new(),
        cover: None,
        bg: None,
        tracks: arr(it, "tracks").filter_map(|t| parse_track(t.pointer("/data/fullModel")?)).collect(),
    };
    match str_of(c, "type").as_str() {
        "wave" => {
            let w = fm.get("wave").unwrap_or(&Value::Null);
            g.kind = Kind::Wave;
            g.target = c.pointer("/data/itemId/seeds/0").and_then(|s| s.as_str()).unwrap_or(crate::api::WAVE).to_string();
            g.title = Some(str_of(w, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| "Моя волна".into());
            g.subtitle = Some(str_of(w, "header")).filter(|t| !t.is_empty()).unwrap_or_else(|| "Моя волна".into());
            g.cover = fm.get("simpleWaveForegroundImageUrl").and_then(|u| u.as_str()).map(String::from);
            g.bg = hex_color(fm.get("simpleWaveBackgroundColor"));
        }
        "album" => {
            if let Some(a) = fm.get("album").and_then(parse_album) {
                g.kind = Kind::Album;
                g.target = format!("album:{}", a.id);
                g.title = a.title;
                g.cover = a.cover;
                g.subtitle = arr(fm, "artists").map(|a| str_of(a, "name")).collect::<Vec<_>>().join(", ");
            }
        }
        "playlist" => {
            if let Some(p) = fm.get("playlist").and_then(parse_playlist) {
                g.kind = Kind::Playlist;
                g.target = format!("playlist:{}:{}", p.uid, p.kind);
                g.title = p.title;
                g.cover = p.covers.into_iter().next();
                g.subtitle = fm.get("tracksCount").and_then(|n| n.as_u64()).map(crate::net::tracks_word).unwrap_or_default();
            }
        }
        "artist" => {
            if let Some(a) = fm.get("artist").and_then(parse_artist) {
                g.kind = Kind::Artist;
                g.target = format!("artist:{}", a.id);
                g.title = a.name;
                g.cover = a.cover;
            }
        }
        _ => {}
    }
    g
}

/// Вкладка и заголовок дня: «Вчера» и «Вчера, 20 ноября», дальше просто «16 ноября».
pub fn day_titles(date: &str, today: NaiveDate) -> (String, String) {
    let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else { return (date.into(), date.into()) };
    let mut full = format!("{} {}", d.day(), MONTHS[d.month0() as usize]);
    if d.year() != today.year() {
        full = format!("{full} {}", d.year());
    }
    let near = match (today - d).num_days() {
        0 => "Сегодня",
        1 => "Вчера",
        2 => "Позавчера",
        _ => return (full.clone(), full),
    };
    (near.into(), format!("{near}, {full}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(day_titles("2026-01-01", today), ("Сегодня".into(), "Сегодня, 1 января".into()));
        assert_eq!(day_titles("2025-12-31", today), ("Вчера".into(), "Вчера, 31 декабря 2025".into()));
        assert_eq!(day_titles("2025-12-20", today), ("20 декабря 2025".into(), "20 декабря 2025".into()));
    }

    #[test]
    fn trims_to_limit() {
        let tracks: Vec<Value> = (0..700).map(|i| json!({ "type": "track", "data": { "itemId": { "trackId": i.to_string() } } })).collect();
        let mut v = json!({ "historyTabs": [
            { "date": "2026-01-02", "items": [{ "tracks": tracks }] },
            { "date": "2026-01-01", "items": [{ "tracks": tracks }, { "tracks": tracks }] },
        ]});
        trim(&mut v);
        let n: Vec<usize> = v.pointer("/historyTabs").unwrap().as_array().unwrap().iter()
            .flat_map(|t| t["items"].as_array().unwrap().iter().map(|i| i["tracks"].as_array().unwrap().len()))
            .collect();
        assert_eq!(n, [700, 300, 0]);
    }
}
