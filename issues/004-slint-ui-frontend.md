# Issue #04: Slint Фронтенд 1-в-1 с SpolzerMod/yamusic-rs

## 1. Метаданные задачи
- **Номер:** #04
- **Целевая ветка разработки:** `Branch: feat/slint-ui-frontend`
- **Статус:** В работе

---

## 2. Контекст и границы задачи (Scope)

### В рамках задачи (In Scope):
- Перенос и адаптация дизайн-системы из `SpolzerMod/yamusic-rs`:
  - Файл стилей `ui/theme.slint` (дизайн-токены, цвета темной темы Яндекс Музыки `#121212`, `#181818`, `#222222`, акцент `#FFCC00`, текст `#FFFFFF`, `#8C8C8C`, скругления, тени, анимации);
  - Модуль типографики и палитры `src/ui/palette.rs`;
  - Набор фирменных векторных SVG иконок (`ui/icons/`: play, pause, prev, next, heart, dislike, shuffle, repeat, volume, mute, search, wave, library, settings);
- Компоненты интерфейса:
  - `Sidebar`: левое навигационное меню (логотип, «Моя волна», «Главное», «Коллекция», поиск, настройки);
  - `PlayerBottomBar`: фиксированная нижняя панель:
    - Обложка, бегущая строка/название, кликабельные артисты;
    - Анимированная кнопка «Нравится» (Heart) и Dislike;
    - Центр: Shuffle, Prev, Play/Pause, Next, Repeat;
    - `SeekBar`: интерактивная полоса воспроизведения с hover-таймкодом и плавной перемоткой;
    - Правый блок: бейдж качества (`FLAC` / `HQ` / `Bit-Perfect`), слайдер громкости со сменой иконки;
  - `WaveWheel` и экран «Моя волна»:
    - Интерактивный селектор вайба (характер, настроение, язык);
  - Экраны `Collection`, `Playlists`, `Search`:
    - Табличный компонент `TrackItem` (номер, обложка, название, артисты, длительность, лайк, меню);
    - Поисковая выдача с вкладками треков, артистов и альбомов;
  - Объединение всех видов в главном окне `ui/main.slint` и привязка моделей в Rust (`src/ui/mod.rs`).

### Вне рамок задачи (Out of Scope):
- Полноэкранный плеер с караоке-текстами LRC (вынесено в Issue #05).
- Динамический k-means экстрактор цветов из обложек треков (вынесено в Issue #09).

---

## 3. Детализированный чек-лист подзадач
- [x] Сформировать файл issues/004-slint-ui-frontend.md с чек-листом.
- [x] Перенести theme.slint, palette.rs и векторные SVG иконки.
- [x] Реализовать нижнюю панель плеера PlayerBottomBar со слайдером SeekBar и контролем громкости.
- [ ] Реализовать экран «Моя волна» с интерактивным выбором вайба WaveWheel.
- [ ] Реализовать боковую панель Sidebar, экраны Collection, Playlists, Search и TrackItem.
- [ ] Интегрировать компоненты в ui/main.slint и связать сигналы с ядром приложения.
- [ ] Провести валидацию проекта: `cargo check`, `cargo clippy`, `cargo test`.

---

## 4. Ожидаемый список атомарных коммитов
1. `feat(ui): add theme styles, typography and SVG icons from yamusic-rs`
2. `feat(ui): implement player bottom bar with seekbar and volume controls`
3. `feat(ui): create My Wave interactive vibe wheel and station view`
4. `feat(ui): implement Collection, Playlist and Search views`

---

## 5. Критерии приемки (Definition of Done)
- Slint-разметка успешно компилируется через `slint-build`.
- Проект собирается и проходит `cargo check`, `cargo clippy -- -D warnings`, `cargo test`.
- Интерфейс соответствует визуальному референсу темной темы Яндекс Музыки.
- Все подзадачи чек-листа переведены в состояние `[x]`.
- Агент останавливается и передает ветку на мердж пользователю.
