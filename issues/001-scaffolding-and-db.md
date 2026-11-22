# Issue #01: Базовый каркас проекта, сборка Slint и схема базы SQLite

## 1. Метаданные задачи
- **Номер:** #01
- **Целевая ветка разработки:** `Branch: feat/scaffolding-and-db`
- **Статус:** В работе

---

## 2. Контекст и границы задачи (Scope)

### В рамках задачи (In Scope):
- Инициализация структуры проекта Rust (`src/main.rs`, `src/core`, `src/db`, `src/ui`, `ui/`).
- Конфигурация `Cargo.toml` с необходимыми зависимостями проекта (Slint, Tokio, Rusqlite, CPAL, Symphonia, Rubato, Biquad, Ringbuf, Reqwest, Serde, Quick-XML, MD-5, Directories, Windows, Global-Hotkey, Tray-Icon, Discord-Rich-Presence, Lofty, Palette).
- Настройка `build.rs` для компиляции разметки Slint и создание базового `ui/main.slint`.
- Проектирование и реализация типов состояния `AppState`, а также каналов сообщений ядра `CoreCommand` и `CoreEvent`.
- Реализация слоя работы с SQLite (`DatabaseRepository` на `rusqlite` bundled) и миграций:
  - Таблица `config`: хранение токенов, настроек аудиовывода, громкости, путей.
  - Таблица `blacklist`: хранение правил фильтрации артистов, треков, стоп-слов, explicit.
  - Таблица `cache_entries`: хранение метаданных и путей кэшированных аудиотреков для LRU.
  - Таблица `eq_presets`: предустановленные и пользовательские настройки 10-полосного эквалайзера.
- Написание unit-тестов для базы данных и валидации состояния.

### Вне рамок задачи (Out of Scope):
- Реализация API-клиента Яндекс Музыки (вынесено в Issue #02).
- Реализация Audio Engine и декодирования (вынесено в Issue #03).
- Полная верстка страниц интерфейса (вынесено в Issue #04).

---

## 3. Детализированный чек-лист подзадач
- [x] Сформировать файл issues/001-scaffolding-and-db.md с чек-листом.
- [x] Настроить Cargo.toml со всеми ключевыми зависимостями проекта.
- [x] Сконфигурировать build.rs для сборки Slint UI файлов и завести минимальный ui/main.slint.
- [ ] Реализовать AppState, каналы сообщений CoreCommand и CoreEvent.
- [ ] Реализовать SQLite миграции: таблицы config, blacklist, cache_entries, eq_presets.
- [ ] Написать unit-тесты для миграций и репозитория БД.
- [ ] Провести финальную валидацию: `cargo check`, `cargo clippy`, `cargo test`.

---

## 4. Ожидаемый список атомарных коммитов
1. `chore: add project dependencies and configure slint build.rs`
2. `feat(core): define AppState, CoreCommand and CoreEvent types`
3. `feat(db): implement SQLite schema migrations and base repository`

---

## 5. Критерии приемки (Definition of Done)
- Проект успешно собирается и проходит `cargo check`.
- Линтер `cargo clippy` отрабатывает чисто без ошибок и предупреждений.
- Unit-тесты `cargo test` проходят успешно (100% pass).
- Все подзадачи чек-листа переведены в состояние `[x]`.
- Агент останавливается и передает ветку на мердж пользователю.
