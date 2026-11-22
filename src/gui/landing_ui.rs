//! «Для вас и Тренды», «Концерты», «Книги и подкасты» и чарт: блоки из скелета
//! складываются в одну модель, каждый блок заполняется, когда приходят его данные.

use super::*;
use crate::landing::{BlockDef, LBlock as Block, LCard as Card2};
use crate::net::group_digits;

#[derive(Default)]
struct LandingState {
    page: Option<Page>,
    skeleton: Option<Arc<Data>>,
    tab: HashMap<String, usize>,
    defs: Vec<BlockDef>,
    shown: Vec<Option<Arc<Data>>>,
    /// Карточки и треки, которые сейчас на экране, — по ним открываются клики.
    cards: Vec<Vec<Card2>>,
    tracks: Vec<Vec<Track>>,
    /// Выбранная вкладка внутри блока («топ / по жанру»), по id блока.
    sub: HashMap<String, usize>,
    model: Option<Rc<VecModel<LBlock>>>,
    chart_shown: Option<Arc<Data>>,
    chart_tracks: Vec<Track>,
}

thread_local! {
    static LS: RefCell<LandingState> = RefCell::new(LandingState::default());
}

const FEED: &str = "/concerts/feed";
/// Жёлтое пятно под датой концерта (ConcertImage_dateBackground).
const DATE_BG: &str = "avatars.mds.yandex.net/get-music-misc/28052/img.69aab8c335547735b2df1c54/%%";

fn color(c: Option<[u8; 3]>) -> Color {
    c.map(rgb).unwrap_or(Color::from_argb_u8(0, 0, 0, 0))
}

/// Карточные блоки, которые рисуются одной лентой.
fn view_kind(kind: &str) -> &str {
    match kind {
        "RECOMMENDED_PLAYLISTS" | "NEW_PLAYLISTS" | "EDITORIAL_COMPILATION" | "NON_MUSIC_EDITORIAL_COMPILATION" | "PERSONAL_PLAYLISTS"
        | "EDITORIAL_ARTISTS" | "PERSONAL_ARTISTS" | "EDITORIAL_ALBUMS" | "NON_MUSIC_CATALOG" => "CARDS",
        k => k,
    }
}

fn is_empty(b: &Block) -> bool {
    b.cards.is_empty() && b.tracks.is_empty() && b.banner.is_none() && b.tabs.iter().all(|t| t.2.is_empty())
}

/// Скрытые блоки для строки в настройках. Пусто — строка не показывается.
pub(super) fn hidden_titles(cfg: &crate::config::Config) -> String {
    cfg.hidden_blocks.iter().map(|h| h.title.as_str()).collect::<Vec<_>>().join(", ")
}

/// У afisha другие размеры картинок: только 100, 200 и 400.
fn afisha_size(px: u32) -> u32 {
    [100, 200, 400].into_iter().find(|&s| s >= px).unwrap_or(400)
}

impl Ctx {
    fn landing_key(def: &BlockDef) -> Option<String> {
        def.uri.as_ref().map(|u| format!("lblock:{}|{}", def.kind, u))
    }

    /// Блоки текущей вкладки: «шапка» скелета, полоса вкладок, блоки вкладки.
    fn landing_defs(&self, page: Page, sk: Option<&crate::landing::Skeleton>) -> Vec<BlockDef> {
        let def = |kind: &str, title: &str, uri: Option<&str>| BlockDef {
            id: kind.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            desc: String::new(),
            uri: uri.map(String::from),
            more: false,
        };
        if page == Page::Concerts {
            return vec![def("CONCERTS_BIG", "Топ для вас", Some(FEED)), def("CONCERTS_GRID", "Концерты для вас", Some(FEED))];
        }
        let Some(sk) = sk else { return Vec::new() };
        let st = self.st.borrow();
        let shown = |d: &&BlockDef| !st.cfg.hidden_blocks.iter().any(|h| h.id == d.id);
        let mut out: Vec<BlockDef> = sk.head.iter().filter(shown).cloned().collect();
        if !sk.tabs.is_empty() {
            let key = format!("{page:?}");
            let sel = LS.with(|l| l.borrow().tab.get(&key).copied()).unwrap_or(sk.selected).min(sk.tabs.len() - 1);
            out.push(def("TABS", "", None));
            out.extend(sk.tabs[sel].blocks.iter().filter(shown).cloned());
        }
        out
    }

    fn landing_hide(self: &Rc<Self>, b: usize) {
        let Some(def) = LS.with(|l| l.borrow().defs.get(b).cloned()) else { return };
        let ui = self.ui();
        let app = ui.global::<App>();
        let Some(row) = app.get_landing_blocks().row_data(b).filter(|r| r.can_hide) else { return };
        {
            let mut st = self.st.borrow_mut();
            if !st.cfg.hidden_blocks.iter().any(|h| h.id == def.id) {
                st.cfg.hidden_blocks.push(crate::config::HiddenBlock { id: def.id.clone(), title: row.title.to_string() });
            }
            st.cfg.save();
            app.set_hidden_blocks(hidden_titles(&st.cfg).into());
        }
        app.set_landing_hover(-1);
        self.refresh_landing();
        self.toast(format!("Блок «{}» скрыт. Вернуть его можно в настройках", row.title));
    }

    pub(super) fn refresh_landing(self: &Rc<Self>) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let page = app.get_page();
        let Some(net) = self.st.borrow().net.clone() else { return };
        let (title, skel) = match page {
            Page::Trends => ("Для вас и Тренды", Some("main")),
            Page::Nonmusic => ("Книги и подкасты", Some("non_music")),
            Page::Concerts => ("Концерты", None),
            _ => return,
        };
        app.set_landing_title(crate::i18n::tr(title).into());
        if page == Page::Concerts && app.get_concert_date_bg().size().width == 0 {
            let size = self.size_for(94.0);
            let w = self.ui.clone();
            self.images.get(&crate::api::pic(DATE_BG, size), size, move |img| {
                if let Some(ui) = w.upgrade() {
                    ui.global::<App>().set_concert_date_bg(img.clone());
                }
            });
        }
        app.set_landing_location(page == Page::Concerts);

        let sk_data = match skel {
            Some(id) => match net.get(&format!("skeleton:{id}")) {
                None => {
                    app.set_landing_loading(true);
                    return;
                }
                Some(Err(e)) => {
                    app.set_landing_loading(false);
                    self.toast(format!("Не удалось загрузить страницу: {e}"));
                    return;
                }
                Some(Ok(d)) => Some(d),
            },
            None => None,
        };
        app.set_landing_loading(false);
        let sk = sk_data.as_deref().and_then(|d| match d {
            Data::Skeleton(s) => Some(s),
            _ => None,
        });

        // Новая страница, новый скелет или другая вкладка — модель собирается заново.
        let defs = self.landing_defs(page, sk);
        let same = LS.with(|l| {
            let l = l.borrow();
            l.page == Some(page)
                && l.model.is_some()
                && match (&l.skeleton, &sk_data) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
                && l.defs.iter().map(|d| &d.id).eq(defs.iter().map(|d| &d.id))
        });
        if !same {
            if let Some(sk) = sk {
                let key = format!("{page:?}");
                let sel = LS.with(|l| l.borrow().tab.get(&key).copied()).unwrap_or(sk.selected).min(sk.tabs.len().saturating_sub(1));
                let tabs: Vec<LTab> = sk.tabs.iter().map(|t| LTab { title: crate::i18n::tr(&t.title).into(), subtitle: t.subtitle.clone().into(), ..Default::default() }).collect();
                let tabs_model = Rc::new(VecModel::from(tabs));
                for (i, t) in sk.tabs.iter().enumerate() {
                    self.load_cover(&tabs_model, i, t.covers.first().map(|s| s.as_str()), 36.0, |r, img| r.cover = img);
                    self.load_cover(&tabs_model, i, t.covers.get(1).map(|s| s.as_str()), 36.0, |r, img| r.cover2 = img);
                }
                app.set_landing_tabs(ModelRc::from(tabs_model));
                app.set_landing_tab(sel as i32);
            } else {
                app.set_landing_tabs(ModelRc::default());
            }
            let rows: Vec<LBlock> = defs
                .iter()
                .map(|d| LBlock {
                    kind: if d.kind == "TABS" {
                        "TABS".into()
                    } else if d.uri.is_some() {
                        "LOADING".into()
                    } else {
                        SharedString::new()
                    },
                    ..Default::default()
                })
                .collect();
            let model = Rc::new(VecModel::from(rows));
            app.set_landing_blocks(ModelRc::from(model.clone()));
            LS.with(|l| {
                let mut l = l.borrow_mut();
                l.page = Some(page);
                l.skeleton = sk_data.clone();
                l.shown = vec![None; defs.len()];
                l.cards = vec![Vec::new(); defs.len()];
                l.tracks = vec![Vec::new(); defs.len()];
                l.defs = defs.clone();
                l.model = Some(model);
            });
        }

        // Первые блоки грузятся сразу, остальные — по мере того, как приходят верхние,
        // чтобы не открывать два десятка соединений за раз.
        let mut pending = 0;
        for (i, def) in defs.iter().enumerate() {
            let Some(key) = Self::landing_key(def) else { continue };
            if pending >= 6 && LS.with(|l| l.borrow().shown.get(i).is_some_and(|s| s.is_none())) {
                // уже ждём шесть блоков — остальные позже
                if net.peek(&key).is_none() {
                    continue;
                }
            }
            match net.get(&key) {
                None => pending += 1,
                Some(Err(_)) => self.landing_set(i, def, None),
                Some(Ok(data)) => {
                    let same = LS.with(|l| l.borrow().shown.get(i).and_then(|s| s.clone()).is_some_and(|s| Arc::ptr_eq(&s, &data)));
                    if !same {
                        LS.with(|l| l.borrow_mut().shown[i] = Some(data.clone()));
                        if let Data::Block(b) = &*data {
                            self.landing_set(i, def, Some(b));
                        }
                    }
                }
            }
        }
    }

    /// Строка модели для блока `i`. `None` — блок не загрузился, он прячется.
    fn landing_set(self: &Rc<Self>, i: usize, def: &BlockDef, b: Option<&Block>) {
        let Some(model) = LS.with(|l| l.borrow().model.clone()) else { return };
        let empty = || LBlock { kind: SharedString::new(), ..Default::default() };
        let Some(b) = b.filter(|b| !is_empty(b)) else {
            model.set_row_data(i, empty());
            return;
        };
        let mut kind = view_kind(&def.kind).to_string();
        let mut cards: Vec<Card2> = b.cards.clone();
        let mut tabs = Vec::new();
        let mut tab = 0;
        if !b.tabs.is_empty() {
            tab = LS.with(|l| l.borrow().sub.get(&def.id).copied()).unwrap_or(0).min(b.tabs.len() - 1);
            cards = b.tabs[tab].2.clone();
            tabs = b.tabs.iter().map(|t| (t.0.clone(), t.1.clone())).collect();
        }
        match def.kind.as_str() {
            "CONCERTS_BIG" => cards.truncate(5),
            "CONCERTS_GRID" => cards = cards.into_iter().skip(5).collect(),
            _ => {}
        }
        if cards.is_empty() && b.tracks.is_empty() && b.banner.is_none() {
            model.set_row_data(i, empty());
            return;
        }
        if kind == "SPECIAL" && b.banner.is_none() {
            kind = String::new();
        }

        // Размер картинки по типу блока.
        let (css, css2): (f32, f32) = match def.kind.as_str() {
            "LIKES_AND_HISTORY" => (58.0, 48.0),
            "ALBUM_PROMO" => (80.0, 321.0),
            "SIMPLE_ALBUM_PROMO" => (64.0, 64.0),
            "NEW_RELEASES" | "EDITORIAL_NEW_RELEASES" => (64.0, 282.0),
            "SETS_BY_WAVES" | "WAVES" => (321.0, 0.0),
            "CONCERTS_BIG" => (333.0, 0.0),
            "CONCERTS_PERSONAL" | "CONCERTS_TOP" | "VIEWED_CONCERTS" | "CONCERTS_GRID" => (265.0, 0.0),
            "DONATIONS" => (156.0, 0.0),
            "MIXES_MUSIC" | "MIXES_GRID" => (115.0, 99.0),
            "EDITORIAL_PROMOTIONS" => (356.0, 0.0),
            "CONTINUE_LISTEN" => (58.0, 0.0),
            "IN_STYLE" => (186.0, 0.0),
            _ => (191.0, 0.0),
        };
        let concert = def.kind.starts_with("CONCERTS") || def.kind == "VIEWED_CONCERTS";
        let rows: Vec<LCard> = cards
            .iter()
            .map(|c| {
                let (month, day, weekday) = c.date.clone().unwrap_or_default();
                LCard {
                    title: c.title.clone().into(),
                    subtitle: c.subtitle.clone().into(),
                    sub2: c.sub2.clone().into(),
                    extra: c.extra.clone().into(),
                    round: c.round,
                    explicit: c.explicit,
                    likes: c.likes.map(group_digits).unwrap_or_default().into(),
                    bg: color(c.bg),
                    bg2: color(c.bg2.or(c.bg)),
                    fg: color(c.fg.or(Some([255, 255, 255]))),
                    month: month.to_uppercase().into(),
                    day: day.into(),
                    weekday: weekday.to_uppercase().into(),
                    pos: if c.pos > 0 { c.pos.to_string().into() } else { SharedString::new() },
                    trend: c.trend,
                    ..Default::default()
                }
            })
            .collect();
        let cards_model = Rc::new(VecModel::from(rows));
        for (k, c) in cards.iter().enumerate() {
            let main = match def.kind.as_str() {
                // Новый релиз: [фото исполнителя, обложка альбома].
                "NEW_RELEASES" | "EDITORIAL_NEW_RELEASES" => c.covers.get(1),
                _ => c.covers.first(),
            };
            if concert {
                if let Some(u) = main.filter(|u| !u.is_empty()) {
                    let px = (css * self.ui().window().scale_factor()).round() as u32;
                    let size = afisha_size(px);
                    let m = cards_model.clone();
                    self.images.get_cancellable(&crate::api::pic(u, size), px, move |img| {
                        if let Some(mut r) = m.row_data(k) {
                            r.cover = img.clone();
                            m.set_row_data(k, r);
                        }
                    });
                }
                continue;
            }
            self.load_cover(&cards_model, k, main.map(|s| s.as_str()).filter(|s| !s.is_empty()), css, |r, img| r.cover = img);
            let second = match def.kind.as_str() {
                "NEW_RELEASES" | "EDITORIAL_NEW_RELEASES" => c.covers.first(),
                "ALBUM_PROMO" | "LIKES_AND_HISTORY" | "MIXES_MUSIC" | "MIXES_GRID" => c.covers.get(1),
                _ => None,
            };
            if css2 > 0.0 {
                self.load_cover(&cards_model, k, second.map(|s| s.as_str()).filter(|s| !s.is_empty()), css2, |r, img| r.cover2 = img);
            }
            if def.kind == "LIKES_AND_HISTORY" {
                self.load_cover(&cards_model, k, c.covers.get(2).map(|s| s.as_str()), css2, |r, img| r.cover3 = img);
            }
        }

        let tabs_model = Rc::new(VecModel::from(tabs.iter().map(|t| LTab { title: t.0.clone().into(), ..Default::default() }).collect::<Vec<_>>()));
        for (k, t) in tabs.iter().enumerate() {
            self.load_cover(&tabs_model, k, t.1.as_deref(), 32.0, |r, img| r.cover = img);
        }

        let tracks = b.tracks.clone();
        let chart = if def.kind == "CHART_TRACKS" {
            let rows = Rc::new(VecModel::from(
                tracks
                    .iter()
                    .zip(b.chart.iter())
                    .map(|(t, (pos, trend))| ChartRow { item: self.track_item(t), pos: pos.to_string().into(), trend: *trend })
                    .collect::<Vec<_>>(),
            ));
            for (k, t) in tracks.iter().enumerate() {
                let Some(uri) = t.cover.as_deref() else { continue };
                let size = self.size_for(40.0);
                let m = rows.clone();
                self.images.get(&crate::api::pic(uri, size), size, move |img| {
                    if let Some(mut r) = m.row_data(k) {
                        r.item.cover = img.clone();
                        m.set_row_data(k, r);
                    }
                });
            }
            ModelRc::from(rows)
        } else {
            ModelRc::default()
        };
        let track_items = if def.kind == "CHART_TRACKS" { ModelRc::default() } else { self.track_model(&tracks) };

        let banner = b.banner.clone().unwrap_or_default();
        let title = match def.kind.as_str() {
            "SPECIAL" => banner.title.clone(),
            // У блока нет заголовка в скелете: оригинал берёт его из словаря.
            "CONTINUE_LISTEN" if def.title.is_empty() => "Продолжить слушать".to_string(),
            _ => def.title.clone(),
        };
        let desc = if def.kind == "SPECIAL" { banner.subtitle.clone() } else { def.desc.clone() };
        let more = def.more || b.target.is_some() || def.kind == "CHART_TRACKS";
        // На странице «Концерты» два блока и есть вся страница, их не прячем.
        let can_hide = !def.id.is_empty() && !title.is_empty() && !matches!(def.kind.as_str(), "CONCERTS_BIG" | "CONCERTS_GRID");
        let row = LBlock {
            kind: kind.into(),
            title: crate::i18n::tr(&title).into(),
            desc: desc.into(),
            more,
            can_hide,
            tabs: ModelRc::from(tabs_model),
            tab: tab as i32,
            cards: ModelRc::from(cards_model),
            tracks: track_items,
            chart,
            ready: true,
            button: banner.button.clone().into(),
            button_bg: color(banner.button_bg.or(Some([255, 255, 0]))),
            button_fg: color(banner.button_fg.or(Some([0, 0, 0]))),
            ink: if self.st.borrow().dark { Color::from_rgb_u8(255, 255, 255) } else { color(banner.text.or(Some([0, 0, 0]))) },
            ad: banner.ad,
            ..Default::default()
        };
        model.set_row_data(i, row);
        LS.with(|l| {
            let mut l = l.borrow_mut();
            if let Some(c) = l.cards.get_mut(i) {
                *c = cards;
            }
            if let Some(t) = l.tracks.get_mut(i) {
                *t = tracks;
            }
        });
        // Обложка в заголовке блока и картинка баннера.
        let cover = if def.kind == "SPECIAL" { banner.image.clone() } else { b.cover.clone() };
        if let Some(uri) = cover {
            let css = if def.kind == "SPECIAL" { 211.0 } else { 58.0 };
            let size = self.size_for(css);
            let m = model.clone();
            self.images.get(&crate::api::pic(&uri, size), size, move |img| {
                if let Some(mut r) = m.row_data(i) {
                    r.cover = img.clone();
                    m.set_row_data(i, r);
                }
            });
        }
    }

    pub(super) fn release_landing(&self) {
        LS.with(|l| {
            let mut l = l.borrow_mut();
            l.model = None;
            l.page = None;
            l.shown.clear();
            l.cards.clear();
            l.tracks.clear();
        });
        let ui = self.ui();
        let app = ui.global::<App>();
        app.set_landing_blocks(ModelRc::default());
        app.set_landing_tabs(ModelRc::default());
    }

    fn landing_select_tab(self: &Rc<Self>, k: usize) {
        let page = self.ui().global::<App>().get_page();
        LS.with(|l| {
            l.borrow_mut().tab.insert(format!("{page:?}"), k);
        });
        self.refresh_landing();
    }

    fn landing_subtab(self: &Rc<Self>, b: usize, k: usize) {
        let Some((def, data)) = LS.with(|l| {
            let l = l.borrow();
            Some((l.defs.get(b)?.clone(), l.shown.get(b)?.clone()?))
        }) else {
            return;
        };
        LS.with(|l| l.borrow_mut().sub.insert(def.id.clone(), k));
        if let Data::Block(blk) = &*data {
            self.landing_set(b, &def, Some(blk));
        }
    }

    fn landing_card(&self, b: usize, i: usize) -> Option<Card2> {
        LS.with(|l| l.borrow().cards.get(b)?.get(i).cloned())
    }

    fn open_landing_target(self: &Rc<Self>, target: &str, title: &str, play: bool) {
        let (kind, arg) = target.split_once(':').unwrap_or((target, ""));
        match kind {
            "likes" => self.navigate(Page::List, Some("likes".into()), true),
            "history" => self.navigate(Page::History, None, true),
            "station" => {
                let src = if arg == crate::api::WAVE { Source::Wave } else { Source::Seed { station: arg.to_string(), header: "Моя волна".into(), title: title.to_string() } };
                self.player.send(Cmd::Wave(src));
                self.start();
            }
            // Адрес из ответа сервера: открываем только https (см. client::browser_url).
            "url" => match crate::client::browser_url(arg) {
                Some(u) => sys::open_url(&u),
                None => crate::config::log(format!("ссылка не открыта: {arg}")),
            },
            "concert" => sys::open_url(&format!("https://music.yandex.ru/concert/{arg}")),
            "album" | "playlist" if play => self.play_target(target, title),
            "artist" if play => {
                if let Some(net) = self.st.borrow().net.clone() {
                    net.play_artist(&self.player, arg, title, None);
                    self.start();
                }
            }
            _ => self.open_target(target, title),
        }
    }

    /// «Слушать» на карточке: альбом или плейлист целиком с первого трека.
    fn play_target(self: &Rc<Self>, target: &str, title: &str) {
        let Some(net) = self.st.borrow().net.clone() else { return };
        let (target, title) = (target.to_string(), title.to_string());
        std::thread::spawn(move || {
            let res = match target.split(':').collect::<Vec<_>>().as_slice() {
                ["album", id] => net.api.album(id).map(|(_, t)| (t, ListKind::Album, id.to_string())),
                ["playlist", uid, kind] => net.api.playlist(uid, kind).map(|(_, t)| (t, ListKind::Playlist, format!("{uid}:{kind}"))),
                _ => return,
            };
            let _ = slint::invoke_from_event_loop(move || {
                with_ctx(|c| match res {
                    Ok((tracks, kind, id)) => c.play_tracks(tracks, 0, kind, &id, &title),
                    Err(e) => c.toast(format!("Не удалось включить: {e}")),
                })
            });
        });
    }

    fn landing_open(self: &Rc<Self>, b: usize, i: usize, play: bool) {
        let kind = LS.with(|l| l.borrow().defs.get(b).map(|d| d.kind.clone())).unwrap_or_default();
        // Треки в колонках: клик — играть с этого трека.
        if matches!(kind.as_str(), "SMART_OPEN_PLAYLIST" | "OPEN_PLAYLIST" | "NON_MUSIC_OPEN_PLAYLIST" | "CHART_TRACKS") {
            let tracks = LS.with(|l| l.borrow().tracks.get(b).cloned()).unwrap_or_default();
            let title = LS.with(|l| l.borrow().defs.get(b).map(|d| d.title.clone())).unwrap_or_default();
            let id = LS.with(|l| {
                let l = l.borrow();
                l.shown.get(b).cloned().flatten().and_then(|d| match &*d {
                    Data::Block(x) => x.target.clone(),
                    _ => None,
                })
            });
            let id = id.unwrap_or_default();
            let id = id.trim_start_matches("playlist:").to_string();
            let id = if kind == "CHART_TRACKS" { "414787002:1076".to_string() } else { id };
            return self.play_tracks(tracks, i, ListKind::Playlist, &id, &title);
        }
        if kind == "SPECIAL" {
            let url = LS.with(|l| {
                let l = l.borrow();
                l.shown.get(b).cloned().flatten().and_then(|d| match &*d {
                    Data::Block(x) => x.banner.as_ref().map(|bn| bn.url.clone()),
                    _ => None,
                })
            });
            if let Some(u) = url.filter(|u| !u.is_empty()) {
                match crate::client::browser_url(&u) {
                    Some(safe) => sys::open_url(&safe),
                    None => crate::config::log(format!("ссылка баннера не открыта: {u}")),
                }
            }
            return;
        }
        let Some(c) = self.landing_card(b, i) else { return };
        let play = play || matches!(kind.as_str(), "SETS_BY_WAVES" | "WAVES" | "NEUROMUSIC");
        self.open_landing_target(&c.target, &c.title, play);
    }

    fn landing_header(self: &Rc<Self>, b: usize) {
        let Some((def, data)) = LS.with(|l| {
            let l = l.borrow();
            Some((l.defs.get(b)?.clone(), l.shown.get(b)?.clone()?))
        }) else {
            return;
        };
        let Data::Block(blk) = &*data else { return };
        if def.kind == "CHART_TRACKS" {
            return self.navigate(Page::Chart, None, true);
        }
        if let Some(t) = &blk.target {
            return self.open_target(t, &def.title);
        }
        // «Смотреть всё»: карточки блока сеткой.
        let cards: Vec<Card> = LS.with(|l| l.borrow().cards.get(b).cloned()).unwrap_or_default()
            .into_iter()
            .map(|c| Card { title: c.title, subtitle: c.subtitle, year: c.sub2, likes: c.likes, covers: c.covers, round: c.round, explicit: c.explicit, target: c.target })
            .collect();
        let key = format!("landing-{}", def.id);
        self.st.borrow_mut().cards.insert(key.clone(), cards);
        self.st.borrow_mut().grid_titles.insert(key.clone(), def.title.clone());
        self.navigate(Page::Grid, Some(key), true);
    }

    pub(super) fn landing_track(&self, block: &str, i: usize) -> Option<Track> {
        if block == "chart-page" {
            return LS.with(|l| l.borrow().chart_tracks.get(i).cloned());
        }
        let b: usize = block.strip_prefix('L')?.parse().ok()?;
        LS.with(|l| l.borrow().tracks.get(b)?.get(i).cloned())
    }

    fn landing_like(self: &Rc<Self>, b: usize, i: usize) {
        let Some(t) = self.landing_track(&format!("L{b}"), i) else { return };
        self.like(t);
        let Some(model) = LS.with(|l| l.borrow().model.clone()) else { return };
        let Some(row) = model.row_data(b) else { return };
        if let Some(mut r) = row.tracks.row_data(i) {
            r.liked = !r.liked;
            row.tracks.set_row_data(i, r);
        }
        if let Some(mut r) = row.chart.row_data(i) {
            r.item.liked = !r.item.liked;
            row.chart.set_row_data(i, r);
        }
    }

    // ---------- чарт ----------

    pub(super) fn refresh_chart(self: &Rc<Self>) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let Some(net) = self.st.borrow().net.clone() else { return };
        match net.get("lblock:CHART_TRACKS|/landing3/chart") {
            None => app.set_chart_loading(true),
            Some(Err(e)) => {
                app.set_chart_loading(false);
                self.toast(format!("Не удалось загрузить чарт: {e}"));
            }
            Some(Ok(data)) => {
                app.set_chart_loading(false);
                if LS.with(|l| l.borrow().chart_shown.as_ref().is_some_and(|d| Arc::ptr_eq(d, &data))) {
                    return;
                }
                let Data::Block(b) = &*data else { return };
                let rows = Rc::new(VecModel::from(
                    b.tracks
                        .iter()
                        .zip(b.chart.iter())
                        .map(|(t, (pos, trend))| ChartRow { item: self.track_item(t), pos: pos.to_string().into(), trend: *trend })
                        .collect::<Vec<_>>(),
                ));
                for (k, t) in b.tracks.iter().enumerate() {
                    let Some(uri) = t.cover.as_deref() else { continue };
                    let size = self.size_for(40.0);
                    let m = rows.clone();
                    self.images.get_cancellable(&crate::api::pic(uri, size), size, move |img| {
                        if let Some(mut r) = m.row_data(k) {
                            r.item.cover = img.clone();
                            m.set_row_data(k, r);
                        }
                    });
                }
                app.set_chart_rows(ModelRc::from(rows));
                LS.with(|l| {
                    let mut l = l.borrow_mut();
                    l.chart_shown = Some(data.clone());
                    l.chart_tracks = b.tracks.clone();
                });
            }
        }
    }

    pub(super) fn release_chart(&self) {
        LS.with(|l| {
            let mut l = l.borrow_mut();
            l.chart_shown = None;
        });
        self.ui().global::<App>().set_chart_rows(ModelRc::default());
    }
}

/// Маски «билета» у карточек концертов и жёлтое пятно под датой — из установленного приложения.
pub(super) fn load_concert_assets(app: &App) {
    let mut asar = crate::assets::Source::open();
    let dir = "app/_next/static/media";
    let path_of = |svg: &str| -> Option<String> {
        let i = svg.find(" d=\"")? + 4;
        let j = svg[i..].find('"')? + i;
        Some(svg[i..j].to_string())
    };
    if let Some(a) = crate::assets::Asar::open() {
        let names = a.list(dir);
        drop(a);
        for name in names {
            let small = name.starts_with("mask.") && name.ends_with(".svg");
            let top = name.starts_with("top-concerts-mask.") && name.ends_with(".svg");
            if !small && !top {
                continue;
            }
            let Some(bytes) = asar.read(&format!("{dir}/{name}")) else { continue };
            let svg = String::from_utf8_lossy(&bytes);
            if top {
                if let Some(p) = path_of(&svg) {
                    app.set_concert_mask_top(p.into());
                }
            } else if svg.contains("width=\"164\"") {
                if let Some(p) = path_of(&svg) {
                    app.set_concert_mask(p.into());
                }
            }
        }
    }
}

pub(super) fn register(app: &App) {
    app.on_landing_select_tab(|k| with_ctx(|c| c.landing_select_tab(k.max(0) as usize)));
    app.on_landing_subtab(|b, k| with_ctx(|c| c.landing_subtab(b.max(0) as usize, k.max(0) as usize)));
    app.on_landing_open(|b, i| with_ctx(|c| c.landing_open(b.max(0) as usize, i.max(0) as usize, false)));
    app.on_landing_play(|b, i| with_ctx(|c| c.landing_open(b.max(0) as usize, i.max(0) as usize, true)));
    app.on_landing_like(|b, i| with_ctx(|c| c.landing_like(b.max(0) as usize, i.max(0) as usize)));
    app.on_landing_header(|b| with_ctx(|c| c.landing_header(b.max(0) as usize)));
    app.on_landing_more(|b| with_ctx(|c| c.landing_header(b.max(0) as usize)));
    app.on_landing_hide(|b| with_ctx(|c| c.landing_hide(b.max(0) as usize)));
    app.on_chart_play(|i| {
        with_ctx(|c| {
            let tracks = LS.with(|l| l.borrow().chart_tracks.clone());
            c.play_tracks(tracks, i.max(0) as usize, ListKind::Playlist, "414787002:1076", "Чарт");
        })
    });
    app.on_chart_like(|i| {
        with_ctx(|c| {
            let Some(t) = LS.with(|l| l.borrow().chart_tracks.get(i.max(0) as usize).cloned()) else { return };
            c.like(t);
            let m = c.ui().global::<App>().get_chart_rows();
            if let Some(mut r) = m.row_data(i as usize) {
                r.item.liked = !r.item.liked;
                m.set_row_data(i as usize, r);
            }
        })
    });
}
