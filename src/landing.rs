//! Страницы из серверных блоков: «Для вас и Тренды», «Концерты», «Книги и подкасты».
//! Скелет (`/landing/skeleton/<id>`) задаёт порядок блоков и вкладки, каждый блок
//! грузится отдельно по своему `source.uri`, как у официального приложения.

use crate::api::{album_kind_word, hex_color, merged, parse_album, parse_artist, parse_playlist, parse_track, str_of, Track};
use serde_json::Value;

/// Блок скелета: что за блок и откуда брать данные.
#[derive(Clone, Debug)]
pub struct BlockDef {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub desc: String,
    pub uri: Option<String>,
    /// Есть ли «Смотреть всё» (стрелка у заголовка).
    pub more: bool,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub title: String,
    pub subtitle: String,
    pub covers: Vec<String>,
    pub blocks: Vec<BlockDef>,
}

#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    /// Блоки над вкладками (у «Книг и подкастов» — «Продолжить слушать»).
    pub head: Vec<BlockDef>,
    pub tabs: Vec<Tab>,
    pub selected: usize,
}

/// Карточка любого блока. Поля, которые блоку не нужны, пустые.
#[derive(Clone, Debug, Default)]
pub struct LCard {
    pub title: String,
    pub subtitle: String,
    pub sub2: String,
    /// Цена концерта, «Поддержать», подпись над названием вайба.
    pub extra: String,
    pub covers: Vec<String>,
    pub round: bool,
    pub explicit: bool,
    pub likes: Option<u64>,
    pub bg: Option<[u8; 3]>,
    pub bg2: Option<[u8; 3]>,
    pub fg: Option<[u8; 3]>,
    /// Концерт: месяц, число, день недели.
    pub date: Option<(String, String, String)>,
    pub pos: u32,
    /// 1 same, 2 up, 3 down, 4 new
    pub trend: i32,
    /// album:ID, playlist:UID:KIND, artist:ID, tag:ID, station:ID, url:…, likes, history
    pub target: String,
}

#[derive(Clone, Debug, Default)]
pub struct LBlock {
    pub cards: Vec<LCard>,
    /// Вкладки внутри блока («топ / по жанру», исполнители «В стиле»): название, обложка, карточки.
    pub tabs: Vec<(String, Option<String>, Vec<LCard>)>,
    pub tracks: Vec<Track>,
    /// Место и движение для треков чарта.
    pub chart: Vec<(u32, i32)>,
    pub cover: Option<String>,
    /// Куда ведёт заголовок блока.
    pub target: Option<String>,
    /// Баннер: заголовок, подзаголовок, кнопка, цвета.
    pub banner: Option<Banner>,
}

#[derive(Clone, Debug, Default)]
pub struct Banner {
    pub title: String,
    pub subtitle: String,
    pub button: String,
    pub image: Option<String>,
    pub button_bg: Option<[u8; 3]>,
    pub button_fg: Option<[u8; 3]>,
    pub text: Option<[u8; 3]>,
    pub url: String,
    pub ad: bool,
}

fn defs(v: &Value) -> Vec<BlockDef> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| {
                    let d = b.get("data").cloned().unwrap_or(Value::Null);
                    Some(BlockDef {
                        id: str_of(b, "id"),
                        kind: b.get("type")?.as_str()?.to_string(),
                        title: str_of(&d, "title").trim().to_string(),
                        desc: str_of(&d, "description"),
                        uri: d.pointer("/source/uri").and_then(|u| u.as_str()).map(String::from),
                        more: d.get("viewAllAction").is_some(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Скелет страницы; вкладки с подписями и обложками приходят отдельным запросом.
pub fn parse_skeleton(v: &Value, tabs: Option<&Value>) -> Skeleton {
    let mut sk = Skeleton::default();
    for b in v.get("blocks").and_then(|b| b.as_array()).into_iter().flatten() {
        if b.get("type").and_then(|t| t.as_str()) != Some("TABS") {
            sk.head.extend(defs(&Value::Array(vec![b.clone()])));
            continue;
        }
        let d = b.get("data").cloned().unwrap_or(Value::Null);
        sk.selected = d.get("selectedTabIndex").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        for t in d.get("tabs").and_then(|t| t.as_array()).into_iter().flatten() {
            let id = str_of(t, "id");
            let extra = tabs.and_then(|x| x.get("tabs")).and_then(|x| x.as_array()).and_then(|a| a.iter().find(|x| str_of(x, "id") == id));
            sk.tabs.push(Tab {
                title: str_of(t, "title"),
                subtitle: extra.map(|e| str_of(e, "subtitle")).unwrap_or_default(),
                covers: extra
                    .and_then(|e| e.get("covers"))
                    .and_then(|c| c.as_array())
                    .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                blocks: defs(t.get("blocks").unwrap_or(&Value::Null)),
            });
        }
    }
    sk
}

fn uri(v: &Value, p: &str) -> Option<String> {
    v.pointer(p).and_then(|u| u.as_str()).filter(|u| !u.is_empty()).map(String::from)
}

pub(crate) const MONTHS: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
const MONTHS_SHORT: [&str; 12] = ["янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
const WEEKDAYS: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];

fn ymd(iso: &str) -> Option<(i32, u32, u32)> {
    let n = |r: std::ops::Range<usize>| iso.get(r).and_then(|s| s.parse::<i64>().ok());
    Some((n(0..4)? as i32, n(5..7)? as u32, n(8..10)? as u32))
}

/// День недели по григорианскому календарю, 0 — понедельник.
fn weekday(y: i32, m: u32, d: u32) -> usize {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let w = (y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + d as i32) % 7;
    ((w + 6) % 7) as usize
}

/// Дата релиза по Москве: в ISO она в UTC, «2026-10-29T21:00» — это 30 октября.
fn release_day(iso: &str) -> String {
    let Some((y, mut m, mut d)) = ymd(iso) else { return String::new() };
    if !(1..=12).contains(&m) {
        return String::new();
    }
    if iso.get(11..13).and_then(|h| h.parse::<u32>().ok()).unwrap_or(0) + 3 >= 24 {
        let days = match m {
            2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        d += 1;
        if d > days {
            d = 1;
            m = m % 12 + 1;
        }
    }
    format!("{d} {}", MONTHS[m as usize - 1])
}

fn concert_date(iso: &str) -> Option<(String, String, String)> {
    let (y, m, d) = ymd(iso)?;
    if !(1..=12).contains(&m) {
        return None;
    }
    Some((MONTHS_SHORT[m as usize - 1].into(), d.to_string(), WEEKDAYS[weekday(y, m, d)].into()))
}

fn album_lcard(v: &Value) -> Option<LCard> {
    let a = parse_album(v)?;
    let explicit = v.get("contentWarning").and_then(|c| c.as_str()) == Some("explicit");
    Some(LCard {
        title: a.title.clone(),
        subtitle: a.artists.clone(),
        sub2: if a.kind == "single" { "Сингл".into() } else { String::new() },
        covers: a.cover.iter().cloned().collect(),
        explicit,
        target: format!("album:{}", a.id),
        ..Default::default()
    })
}

/// Элемент `items` блока: альбом, плейлист, исполнитель, подборка, концерт.
fn item_card(it: &Value) -> Option<LCard> {
    let d = it.get("data").unwrap_or(it);
    match it.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "album_item" => album_lcard(&merged(d, "album")?),
        "non_music_album_item" => {
            let mut c = album_lcard(&merged(d, "album")?)?;
            c.likes = d.get("likesCount").and_then(|x| x.as_u64());
            c.sub2 = String::new();
            Some(c)
        }
        "liked_playlist_item" | "playlist_item" | "personal_playlist_item" => {
            let p = parse_playlist(&merged(d, "playlist")?)?;
            Some(LCard {
                title: p.title.clone(),
                subtitle: str_of(d, "description"),
                likes: d.get("likesCount").and_then(|x| x.as_u64()),
                covers: p.covers.clone(),
                target: format!("playlist:{}:{}", p.uid, p.kind),
                ..Default::default()
            })
        }
        "artist_item" => {
            let a = parse_artist(d.get("artist")?)?;
            Some(LCard { title: a.name, covers: a.cover.into_iter().collect(), round: true, target: format!("artist:{}", a.id), ..Default::default() })
        }
        "mix_card_item" | "mixes_grid_item" | "mix_item" => Some(LCard {
            title: str_of(d, "title"),
            covers: d
                .get("covers")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|c| c.get("uri")?.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            bg: hex_color(d.pointer("/covers/1/color")).or_else(|| hex_color(d.pointer("/covers/0/color"))),
            target: format!("tag:{}", d.pointer("/action/weblink").and_then(|w| w.as_str()).unwrap_or("").trim_start_matches("/tag/")),
            ..Default::default()
        }),
        "concert_item" => {
            let mut c = concert_card(d.get("concert")?)?;
            if let Some(p) = d.get("minPrice") {
                let v = p.get("value").and_then(|x| x.as_u64()).unwrap_or(0);
                c.extra = format!("от {} {}", crate::net::group_digits(v), str_of(p, "currencySymbol"));
            }
            Some(c)
        }
        "track_item" => {
            let t = parse_track(d.get("track")?)?;
            Some(LCard {
                title: t.title.clone(),
                subtitle: t.artists.clone(),
                covers: t.cover.iter().cloned().collect(),
                explicit: t.explicit,
                target: t.album_id.as_ref().map(|a| format!("album:{a}")).unwrap_or_default(),
                ..Default::default()
            })
        }
        _ => None,
    }
}

fn concert_card(c: &Value) -> Option<LCard> {
    let rating = str_of(c, "contentRating");
    Some(LCard {
        title: str_of(c, "concertTitle"),
        subtitle: [String::from("Концерт"), rating].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • "),
        sub2: str_of(c, "city"),
        covers: uri(c, "/cover/uri").into_iter().collect(),
        date: concert_date(&str_of(c, "datetime")),
        target: format!("concert:{}", str_of(c, "id")),
        ..Default::default()
    })
}

/// Карточка нового релиза: исполнитель в круге, под ним плашка с альбомом.
fn release_lcard(r: &Value) -> Option<LCard> {
    let al = r.get("album")?;
    let a = parse_album(al)?;
    let names: Vec<String> = r
        .get("artists")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("name")?.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let kind = al.get("albumType").or_else(|| al.get("type")).and_then(|k| k.as_str()).unwrap_or("");
    let date = release_day(&str_of(r, "releaseDate"));
    Some(LCard {
        title: a.title.clone(),
        subtitle: names.join(", "),
        sub2: [album_kind_word(kind).to_lowercase(), date].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • "),
        covers: [uri(r, "/cover/uri"), a.cover.clone()].into_iter().flatten().collect(),
        explicit: al.get("contentWarning").and_then(|c| c.as_str()) == Some("explicit"),
        bg: hex_color(al.pointer("/cover/color")).or_else(|| hex_color(r.pointer("/cover/color"))),
        bg2: hex_color(r.pointer("/cover/color")),
        target: format!("album:{}", a.id),
        ..Default::default()
    })
}

/// Кнопка вайба: «Сет Моей волны под занятие / Бег».
fn vibe_card(it: &Value) -> Option<LCard> {
    Some(LCard {
        title: str_of(it, "title"),
        extra: str_of(it, "header"),
        covers: uri(it, "/backgroundImageUrl").into_iter().collect(),
        bg: hex_color(it.pointer("/colors/average")),
        fg: hex_color(it.pointer("/colors/waveText")),
        target: format!("station:{}", str_of(it, "stationId")),
        ..Default::default()
    })
}

fn tracks_of(v: &Value) -> Vec<Track> {
    v.as_array().map(|a| a.iter().filter_map(|t| parse_track(t.get("track").unwrap_or(t))).collect()).unwrap_or_default()
}

fn trend(p: &str) -> i32 {
    match p {
        "up" => 2,
        "down" => 3,
        "new" => 4,
        _ => 1,
    }
}

pub fn parse_block(kind: &str, v: &Value) -> LBlock {
    let mut b = LBlock::default();
    let items = |key: &str| v.get(key).and_then(|i| i.as_array()).cloned().unwrap_or_default();
    match kind {
        "LIKES_AND_HISTORY" => {
            if let Some(f) = v.get("favorites") {
                let count = f.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
                let mut covers: Vec<String> = uri(f, "/cover/uri").into_iter().collect();
                covers.extend(f.get("trackCovers").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| uri(c, "/uri")));
                b.cards.push(LCard {
                    title: str_of(f, "title"),
                    subtitle: crate::net::tracks_word(count),
                    covers,
                    target: "likes".into(),
                    ..Default::default()
                });
            }
            if let Some(h) = v.get("history") {
                let sub: Vec<String> =
                    h.get("subtitleElements").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                let mut covers = vec![String::new()];
                covers.extend(h.get("trackCovers").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| uri(c, "/uri")));
                b.cards.push(LCard { title: str_of(h, "title"), subtitle: sub.join(", "), covers, target: "history".into(), ..Default::default() });
            }
        }
        "ALBUM_PROMO" | "SIMPLE_ALBUM_PROMO" => {
            for it in items("albumBanners") {
                let Some(mut c) = it.get("album").and_then(|a| merged(&it, "album").or(Some(a.clone()))).as_ref().and_then(album_lcard) else { continue };
                // [обложка альбома, фото исполнителя]
                c.covers = vec![
                    uri(&it, "/album/cover/uri").or_else(|| uri(&it, "/album/coverUri")).unwrap_or_default(),
                    uri(&it, "/cover/uri").unwrap_or_default(),
                ];
                c.bg = hex_color(it.pointer("/album/cover/color")).or_else(|| hex_color(it.pointer("/cover/color")));
                b.cards.push(c);
            }
        }
        "SPECIAL" => {
            let theme = v.get("lightTheme").unwrap_or(v);
            b.banner = Some(Banner {
                title: str_of(v, "title"),
                subtitle: str_of(v, "subtitle"),
                button: str_of(v, "buttonTitle"),
                image: uri(theme, "/imageUrl").or_else(|| uri(v, "/imageUrl")),
                button_bg: hex_color(theme.get("buttonColor")),
                button_fg: hex_color(theme.get("buttonTextColor")),
                text: hex_color(theme.get("textColor")),
                url: v.pointer("/action/weblink").and_then(|w| w.as_str()).map(webview_url).unwrap_or_default(),
                ad: v.get("advDisclaimer").and_then(|a| a.as_str()).is_some_and(|a| !a.is_empty()),
            });
        }
        "SETS_BY_WAVES" | "WAVES" => {
            for t in items("waves") {
                let cards = t.get("items").and_then(|i| i.as_array()).into_iter().flatten().filter_map(vibe_card).collect();
                b.tabs.push((str_of(&t, "title"), None, cards));
            }
        }
        "NEW_RELEASES" | "EDITORIAL_NEW_RELEASES" => b.cards = items("newReleases").iter().filter_map(release_lcard).collect(),
        "IN_STYLE" => {
            for t in items("inStyleTabs") {
                let cards = t
                    .get("items")
                    .and_then(|i| i.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|it| album_lcard(&merged(it, "album")?))
                    .collect();
                b.tabs.push((str_of(&t, "title"), uri(&t, "/cover/uri"), cards));
            }
        }
        "CONCERTS_PERSONAL" | "CONCERTS_TOP" | "VIEWED_CONCERTS" => {
            b.cards = items("concerts").iter().filter_map(concert_card).collect();
            if b.cards.is_empty() {
                b.cards = items("items").iter().filter_map(item_card).collect();
            }
        }
        "SMART_OPEN_PLAYLIST" | "OPEN_PLAYLIST" | "NON_MUSIC_OPEN_PLAYLIST" => {
            b.tracks = tracks_of(v.get("tracks").unwrap_or(&Value::Null));
            b.cover = uri(v, "/cover/uri").or_else(|| uri(v, "/playlist/cover/uri"));
            if let Some(p) = v.get("playlist").and_then(parse_playlist) {
                b.target = Some(format!("playlist:{}:{}", p.uid, p.kind));
            }
        }
        "CHART_TRACKS" => {
            for t in v.pointer("/chart/tracks").and_then(|t| t.as_array()).into_iter().flatten() {
                let Some(tr) = parse_track(t.get("track").unwrap_or(t)) else { continue };
                let pos = t.pointer("/chart/position").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
                let progress = t.pointer("/chart/progress").and_then(|p| p.as_str()).unwrap_or("same");
                b.tracks.push(tr);
                b.chart.push((pos, trend(progress)));
            }
            b.target = Some("chart".into());
        }
        "DONATIONS" => {
            for it in items("donations") {
                let d = it.get("data").unwrap_or(&it);
                let Some(a) = d.get("artist").and_then(parse_artist) else { continue };
                b.cards.push(LCard {
                    title: a.name,
                    subtitle: d.pointer("/goal/title").and_then(|g| g.as_str()).unwrap_or("").into(),
                    covers: a.cover.into_iter().collect(),
                    round: true,
                    target: format!("url:{}", str_of(d, "tipUrl")),
                    ..Default::default()
                });
            }
        }
        "NEUROMUSIC" => {
            for it in items("items") {
                b.cards.push(LCard {
                    title: str_of(&it, "title"),
                    bg: hex_color(it.pointer("/style/backgroundColor")),
                    fg: hex_color(it.pointer("/style/titleColor")),
                    target: format!("station:{}", str_of(&it, "stationId")),
                    ..Default::default()
                });
            }
        }
        "CHART_ALBUMS" => {
            for it in items("items") {
                let d = it.get("data").unwrap_or(&it);
                let Some(al) = merged(d, "album") else { continue };
                let Some(mut c) = album_lcard(&al) else { continue };
                c.pos = d.pointer("/chart/position").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
                c.trend = trend(d.pointer("/chart/progress").and_then(|p| p.as_str()).unwrap_or("same"));
                c.likes = d.get("likesCount").or_else(|| al.get("likesCount")).and_then(|l| l.as_u64());
                c.sub2 = String::new();
                b.cards.push(c);
            }
        }
        "EDITORIAL_PROMOTIONS" => {
            for d in items("promotions") {
                b.cards.push(LCard {
                    title: str_of(&d, "title"),
                    subtitle: str_of(&d, "subtitle"),
                    covers: uri(&d, "/imageUrl").into_iter().collect(),
                    target: d.pointer("/action/weblink").and_then(|w| w.as_str()).map(web_target).unwrap_or_default(),
                    ..Default::default()
                });
            }
        }
        "CONTINUE_LISTEN" => {
            if let Some(lp) = v.pointer("/lastPlayed/data") {
                let al = lp.get("album");
                let t = lp.get("track").and_then(parse_track);
                let left = lp.get("durationLeft").and_then(|x| x.as_u64()).unwrap_or(0);
                b.cards.push(LCard {
                    title: t.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
                    subtitle: format!("{} минут", left.div_ceil(60)),
                    covers: al.and_then(|a| uri(a, "/cover/uri")).into_iter().collect(),
                    bg: al.and_then(|a| hex_color(a.pointer("/cover/derivedColors/average"))),
                    target: al.and_then(|a| a.get("id")).map(|id| format!("album:{}", id.to_string().trim_matches('"'))).unwrap_or_default(),
                    extra: "last".into(),
                    ..Default::default()
                });
            }
            for (key, bg) in [("bookshelf", [0x60, 0x35, 0x7e]), ("newEpisodes", [0x91, 0x20, 0x61])] {
                let Some(x) = v.get(key) else { continue };
                b.cards.push(LCard {
                    title: str_of(x, "title").split_whitespace().collect::<Vec<_>>().join(" "),
                    covers: x.get("covers").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| uri(c, "/uri")).collect(),
                    bg: Some(bg),
                    target: x.pointer("/action/weblink").and_then(|w| w.as_str()).map(web_target).unwrap_or_default(),
                    ..Default::default()
                });
            }
        }
        _ => {
            b.cards = items("items").iter().filter_map(item_card).collect();
        }
    }
    b
}

/// Ссылка из блока: внутренняя страница приложения или веб-страница.
fn web_target(link: &str) -> String {
    let q = |name: &str| {
        link.split(['?', '&'])
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .map(|v| v.to_string())
    };
    if link.starts_with("/album") {
        if let Some(id) = q("albumId").or_else(|| link.strip_prefix("/album/").map(|r| r.split(['/', '?']).next().unwrap_or("").to_string())) {
            return format!("album:{id}");
        }
    }
    if link.starts_with("/artist") {
        if let Some(id) = q("artistId").or_else(|| link.strip_prefix("/artist/").map(|r| r.split(['/', '?']).next().unwrap_or("").to_string())) {
            return format!("artist:{id}");
        }
    }
    if link.starts_with("/webview") {
        return format!("url:{}", webview_url(link));
    }
    // Относительная ссылка — только от «/»: склейка с `.evil.com/` или `@evil.com/` увела бы
    // на чужой хост. Остальное — как есть; схему и хост проверяет `client::browser_url`
    // перед открытием.
    if link.starts_with('/') {
        return format!("url:https://music.yandex.ru{link}");
    }
    format!("url:{link}")
}

/// `/webview?url=https%3A%2F%2F…` → адрес страницы.
fn webview_url(link: &str) -> String {
    let Some(enc) = link.split(['?', '&']).find_map(|p| p.strip_prefix("url=")) else {
        // Как в web_target: к хосту приклеиваем только путь от «/».
        return if link.starts_with('/') { format!("https://music.yandex.ru{link}") } else { link.to_string() };
    };
    let mut out = Vec::new();
    let b = enc.as_bytes();
    // По байтам, не срезами строки: срез `&enc[i + 1..i + 3]` посреди многобайтного символа
    // (`%aЖ`) паниковал, а с panic = "abort" это падение всего приложения.
    let hex = |c: u8| (c as char).to_digit(16);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(release_day("2026-10-29T21:00:00+00:00"), "30 октября");
        assert_eq!(concert_date("2026-11-13T20:00:00+10:00"), Some(("ноя".into(), "13".into(), "пт".into())));
        assert_eq!(concert_date("2027-01-04T19:00:00+03:00"), Some(("янв".into(), "4".into(), "пн".into())));
    }

    #[test]
    fn links() {
        assert_eq!(web_target("/album?albumId=5&utm=1"), "album:5");
        assert_eq!(webview_url("/webview?url=https%3A%2F%2Fmusic.yandex.ru%2Fpromo"), "https://music.yandex.ru/promo");
        assert_eq!(web_target("/promo"), "url:https://music.yandex.ru/promo");
        assert_eq!(web_target(".evil.com/"), "url:.evil.com/");
        assert_eq!(web_target("@evil.com/"), "url:@evil.com/");
        assert_eq!(webview_url("/promo"), "https://music.yandex.ru/promo");
        assert_eq!(webview_url(".evil.com/"), ".evil.com/");
        assert_eq!(webview_url("https://music.yandex.ru/x"), "https://music.yandex.ru/x");
    }

    #[test]
    fn webview_decoding() {
        // Многобайтный символ сразу после «%» — без паники, как есть.
        assert_eq!(webview_url("/webview?url=%aЖ"), "%aЖ");
        assert_eq!(webview_url("/webview?url=%Ж1"), "%Ж1");
        assert_eq!(webview_url("/webview?url=a%41"), "aA");
        assert_eq!(webview_url("/webview?url=%+1"), "% 1");
        assert_eq!(webview_url("/webview?url=%4"), "%4");
        assert_eq!(webview_url("/webview?url=%D0%96"), "Ж");
    }
}
