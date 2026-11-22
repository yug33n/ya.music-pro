//! Страница «История»: дни, группы и треки лежат одной моделью строк, а рисуются только
//! те, что попали на экран.

use super::*;
use crate::history::{Day, Kind};

#[derive(Clone, Copy)]
enum Row {
    Day,
    Group(usize, usize),
    Track(usize, usize, usize),
}

#[derive(Default)]
struct HistState {
    shown: Option<Arc<Data>>,
    model: Option<Rc<VecModel<HistRow>>>,
    rows: Vec<Row>,
    ys: Vec<f32>,
    ends: Vec<f32>,
    tabs: Vec<f32>,
    /// Прокрутка и высота окна прокрутки.
    view: (f32, f32),
}

thread_local! {
    static HS: RefCell<HistState> = RefCell::new(HistState { view: (0.0, 800.0), ..Default::default() });
}

/// Запас строк за краем экрана, в пикселях.
const MARGIN: f32 = 200.0;

fn days(data: &Data) -> &[Day] {
    match data {
        Data::History(d) => d,
        _ => &[],
    }
}

impl Ctx {
    pub(super) fn enter_history(self: &Rc<Self>) {
        if let Some(net) = self.st.borrow().net.clone() {
            if net.peek("history").is_some() {
                net.refresh("history");
            }
        }
        self.refresh_history();
    }

    pub(super) fn refresh_history(self: &Rc<Self>) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let Some(net) = self.st.borrow().net.clone() else { return };
        match net.get("history") {
            None => app.set_hist_loading(HS.with(|h| h.borrow().shown.is_none())),
            Some(Err(e)) => {
                app.set_hist_loading(false);
                self.toast(format!("Не удалось загрузить историю: {e}"));
            }
            Some(Ok(data)) => {
                app.set_hist_loading(false);
                if HS.with(|h| h.borrow().shown.as_ref().is_some_and(|d| Arc::ptr_eq(d, &data))) {
                    return;
                }
                self.show_history(data);
            }
        }
    }

    fn show_history(self: &Rc<Self>, data: Arc<Data>) {
        let today = chrono::Local::now().date_naive();
        let (mut items, mut rows, mut ys, mut ends, mut tabs, mut tab_titles) = (vec![], vec![], vec![], vec![], vec![], vec![]);
        let mut push = |r: HistRow, row: Row, h: f32| {
            ys.push(r.y);
            ends.push(r.y + h);
            items.push(r);
            rows.push(row);
        };
        let mut y = 16.0;
        for (di, d) in days(&data).iter().enumerate() {
            if di > 0 {
                y += 32.0;
            }
            let (tab, title) = crate::history::day_titles(&d.date, today);
            tabs.push(y);
            tab_titles.push(HistTab { title: tab.into(), y });
            push(HistRow { kind: 0, y, title: title.into(), ..Default::default() }, Row::Day, 26.0);
            y += 42.0;
            for (gi, g) in d.groups.iter().enumerate() {
                if gi > 0 {
                    y += 24.0;
                }
                if g.kind != Kind::Other {
                    let label = match g.kind {
                        Kind::Album => "Альбом",
                        Kind::Playlist => "Плейлист",
                        Kind::Artist => "Исполнитель",
                        _ => "",
                    };
                    let h = if label.is_empty() { 58.0 } else { 88.0 };
                    let row = HistRow {
                        kind: 1,
                        y,
                        title: g.title.as_str().into(),
                        subtitle: g.subtitle.as_str().into(),
                        label: label.into(),
                        round: matches!(g.kind, Kind::Wave | Kind::Artist),
                        bg: g.bg.map(rgb).unwrap_or_default(),
                        arrow: g.kind != Kind::Wave,
                        ..Default::default()
                    };
                    push(row, Row::Group(di, gi), h);
                    y += h + 12.0;
                }
                let album = g.kind == Kind::Album;
                for (ti, t) in g.tracks.iter().enumerate() {
                    let row = HistRow {
                        kind: if album { 3 } else { 2 },
                        y,
                        pos: t.position.map(|p| p.to_string()).unwrap_or_default().into(),
                        item: self.track_item(t),
                        ..Default::default()
                    };
                    push(row, Row::Track(di, gi, ti), 56.0);
                    y += 56.0;
                }
            }
        }
        let model = Rc::new(VecModel::from(items));
        let ui = self.ui();
        let app = ui.global::<App>();
        app.set_hist_count(0);
        app.set_hist_rows(ModelRc::from(model.clone()));
        app.set_hist_tabs(ModelRc::from(Rc::new(VecModel::from(tab_titles))));
        app.set_hist_height(y + 40.0);
        HS.with(|h| {
            let mut h = h.borrow_mut();
            h.shown = Some(data);
            h.model = Some(model);
            (h.rows, h.ys, h.ends, h.tabs) = (rows, ys, ends, tabs);
        });
        self.hist_window();
    }

    pub(super) fn release_history(&self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        app.set_hist_count(0);
        app.set_hist_rows(ModelRc::default());
        app.set_hist_tabs(ModelRc::default());
        HS.with(|h| {
            let view = h.borrow().view;
            *h.borrow_mut() = HistState { view: (0.0, view.1), ..Default::default() };
        });
    }

    /// Какие строки на экране и какой день выбран во вкладках.
    fn hist_window(&self) {
        let (first, count, tab) = HS.with(|h| {
            let h = h.borrow();
            let (top, height) = h.view;
            let first = h.ends.partition_point(|&e| e < top - MARGIN);
            let last = h.ys.partition_point(|&y| y < top + height + MARGIN);
            let tab = h.tabs.partition_point(|&y| y <= top + 40.0).saturating_sub(1);
            (first, last.saturating_sub(first), tab)
        });
        let ui = self.ui();
        let app = ui.global::<App>();
        if app.get_hist_first() != first as i32 || app.get_hist_count() != count as i32 {
            app.set_hist_first(first as i32);
            app.set_hist_count(count as i32);
        }
        app.set_hist_tab(tab as i32);
    }

    fn hist_row(&self, i: usize) -> Option<(Arc<Data>, Row)> {
        HS.with(|h| {
            let h = h.borrow();
            Some((h.shown.clone()?, *h.rows.get(i)?))
        })
    }

    pub(super) fn history_track(&self, i: usize) -> Option<Track> {
        let (data, Row::Track(d, g, t)) = self.hist_row(i)? else { return None };
        days(&data).get(d)?.groups.get(g)?.tracks.get(t).cloned()
    }

    fn hist_cover(self: &Rc<Self>, i: usize) {
        let Some((data, row)) = self.hist_row(i) else { return };
        let Some(model) = HS.with(|h| h.borrow().model.clone()) else { return };
        let has = |r: &HistRow| if r.kind == 1 { r.cover.size().width > 0 } else { r.item.cover.size().width > 0 };
        if model.row_data(i).is_none_or(|r| has(&r)) {
            return;
        }
        let days = days(&data);
        let (uri, size) = match row {
            Row::Group(d, g) => (days[d].groups[g].cover.clone(), self.size_for(58.0)),
            Row::Track(d, g, t) if days[d].groups[g].kind != Kind::Album => (days[d].groups[g].tracks[t].cover.clone(), self.size_for(40.0)),
            _ => return,
        };
        let Some(uri) = uri else { return };
        self.images.get_cancellable(&crate::api::pic(&uri, size), size, move |img| {
            if let Some(mut r) = model.row_data(i) {
                match r.kind {
                    1 => r.cover = img.clone(),
                    _ => r.item.cover = img.clone(),
                }
                model.set_row_data(i, r);
            }
        });
    }

    fn hist_play(self: &Rc<Self>, i: usize) {
        let Some((data, Row::Track(d, g, t))) = self.hist_row(i) else { return };
        let g = &days(&data)[d].groups[g];
        let (kind, id) = match g.kind {
            Kind::Album => (ListKind::Album, g.target.trim_start_matches("album:").to_string()),
            Kind::Playlist => (ListKind::Playlist, g.target.trim_start_matches("playlist:").to_string()),
            _ => (ListKind::Tracks, String::from("history")),
        };
        self.play_tracks(g.tracks.clone(), t, kind, &id, &g.title);
    }

    fn hist_like(self: &Rc<Self>, i: usize) {
        let Some(t) = self.history_track(i) else { return };
        self.like(t);
        let Some(model) = HS.with(|h| h.borrow().model.clone()) else { return };
        if let Some(mut r) = model.row_data(i) {
            r.item.liked = !r.item.liked;
            model.set_row_data(i, r);
        }
    }

    fn hist_open(self: &Rc<Self>, i: usize) {
        let Some((data, Row::Group(d, g))) = self.hist_row(i) else { return };
        let g = &days(&data)[d].groups[g];
        if g.kind == Kind::Wave {
            let src = if g.target == crate::api::WAVE {
                Source::Wave
            } else {
                Source::Seed { station: g.target.clone(), header: g.subtitle.clone(), title: g.title.clone() }
            };
            self.player.send(Cmd::Wave(src));
            self.start();
        } else {
            self.open_target(&g.target, &g.title);
        }
    }
}

pub(super) fn register(app: &App) {
    app.on_hist_scrolled(|top, height| {
        with_ctx(|c| {
            HS.with(|h| h.borrow_mut().view = (top, height));
            c.hist_window();
        })
    });
    app.on_hist_cover(|i| with_ctx(|c| c.hist_cover(i.max(0) as usize)));
    app.on_hist_play(|i| with_ctx(|c| c.hist_play(i.max(0) as usize)));
    app.on_hist_like(|i| with_ctx(|c| c.hist_like(i.max(0) as usize)));
    app.on_hist_open(|i| with_ctx(|c| c.hist_open(i.max(0) as usize)));
}
