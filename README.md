# Solana Momentum Bot

Адаптивный snipe-бот для ранних движений на мемкоинах на Solana (PumpSwap, Raydium, Meteora и др.). Активная разработка — на **Rust** (`crates/`); `src/`, `test/`, `package.json` в корне — это архивный JS-референс Этапа 0, оставленный как читаемая спецификация. Их логика перенесена в `crates/core` один в один, с тем же набором тестов, и дальнейшая разработка идёт только в Rust.

## Структура

- `crates/core` — портированная логика Этапа 0: схема событий (`domain`), риск-движок (`risk_engine`, `scoring_config`), контракт venue-адаптера с fail-closed halt при несовпадении версии (`adapter_contract`), детерминированный replay (`replay`), дедупликация двух независимых Geyser-потоков (`dedup`), запись событий в NDJSON (`recorder`).
- `crates/pump` — Pump bonding curve: декодер `BondingCurve` и constant-product quote-математика, сверенные с реальными данными mainnet (см. `crates/pump/src/lib.rs` и `crates/pump/idl/pump.json`).
- `crates/pumpswap` — PumpSwap AMM: декодер `Pool` и constant-product quote-математика, тоже сверенные с реальными сделками (`crates/pumpswap/idl/pump_amm.json`).
- `crates/token2022` — проверка опасных расширений Token-2022 (transfer fee/hook, permanent delegate, non-transferable, default-frozen) поверх официального крейта `spl-token-2022`, без ручного разбора TLV.
- `crates/ingest` — склеивающий слой над `core`/`pump`/`pumpswap`/`token2022`: собирает decoded `Candidate` + результат инспекции минта в `core::domain::Event`, который уже понимает risk-engine.
- `docs/VENUE_ADAPTER.md` — контракт venue-адаптера, теперь полностью реализован как Rust `trait` (`crates/core/src/adapter_contract.rs::VenueAdapter`) и используется обоими venue-адаптерами (`crates/pump/src/adapter.rs`, `crates/pumpswap/src/adapter.rs`).
- `src/`, `test/`, `package.json` — архив: JS-версия Этапа 0, из которой был сделан перенос. Не развивается дальше.

## Что уже покрыто (`crates/core`)

- Нормализованные события: `TokenCreated`, `MetadataCreated`, `MintTo`, `AuthorityChanged`, `PoolCreated`, `CurveCreated`, `Buy`, `Sell`, `TokenTransfer`, `Graduation`, `Migration`, `LiquidityAdded`, `LiquidityRemoved`, `HolderSnapshot`, `NarrativeUpdated` — типобезопасный `enum EventPayload`, а не произвольный объект: несуществующего события или события без обязательных полей просто нельзя сконструировать.
- Снимок риска после каждого события: `safety_score`, `creator_score`, `demand_score`, `narrative_score`, концентрация держателей, давление продаж, ликвидность на выход, вероятность graduation.
- Жёсткие стоп-факторы (`HardBlock`): активные mint/freeze authority, post-launch mint, transfer hook/fee, неподдерживаемая token-программа, удалённая ликвидность.
- Два входа: `confirmed_entry` для обычного сильного кандидата и уменьшенный `probe_entry` для сильного нарратива с неизвестным создателем.
- Контракт venue-адаптера (`AdapterRegistry`) с фиксированной версией program layout/IDL и `AdapterVersionMismatch` при несовпадении.
- Replay сортирует события по slot, времени наблюдения, signature и instruction index — одинаковый набор данных даёт одинаковый результат независимо от порядка поступления.
- Скоринговые пороги — в версионируемом `scoring_config::DEFAULT_SCORING_CONFIG`.
- Дедупликация двух независимых Geyser/Yellowstone-потоков по паре `(venue, signature, instructionIndex)`.

## Этап 1: Pump и PumpSwap (`crates/pump`, `crates/pumpswap`, `crates/token2022`)

Математика в обоих крейтах сверена с реальными транзакциями mainnet, а не только с документацией — см. doc-комментарии в начале каждого `lib.rs` и тесты в `tests/`, где зафиксированы конкретные подписи транзакций и точные цифры, с которыми сверялась формула.

Ключевые находки в процессе:
- **Реальный протокол Pump заметно сложнее старой документации**: комиссии считаются не по статичной формуле, а через CPI в отдельную недокументированную программу; наблюдалось расхождение между её заявленными и реально списанными bps. Поэтому `quote_buy`/`quote_sell` в обоих крейтах возвращают **чистую сумму сделки без комиссии** — для replay комиссия берётся из реального исторического события, а не пересчитывается.
- **Fail-closed на нестандартные варианты**: mayhem mode, cashback-монеты, произвольный quote_mint (Pump) и boosted-пулы с `virtual_quote_reserves != 0` (PumpSwap) — не оцениваются, `is_standard()` явно отказывает, вместо того чтобы тихо посчитать неверно. Один из boosted-пулов при проверке дал ~9% расхождения с обычной формулой — это и есть причина отказа, не гипотетическая осторожность.
- Формулы обеих сторон (buy и sell) сверены с реальными сделками с точностью до лампорта/минимальной единицы, не приблизительно.
- Токены Pump сейчас по умолчанию чеканятся через Token-2022, не через старый Token program — отсюда важность `crates/token2022` уже на этом этапе, не как редкий кейс.

## Запуск проверок

```bash
cargo test --workspace
```

## Границы

Числа в скоринге — начальные исследовательские пороги, а не торговая рекомендация. Их можно менять только через версионируемую конфигурацию после walk-forward бэктеста. Комиссии Pump/PumpSwap не пересчитываются локально — только читаются из реальных исторических событий при replay (см. выше).

`crates/ingest` пока умеет собирать только `TokenCreated` (Pump) — безопасность минта проверяется один раз, в момент создания. Если авторитет комиссии/заморозки меняет параметры Token-2022 расширений уже после запуска (например, поднимает `transfer_fee_bps` или включает `DefaultAccountState = Frozen` постфактум), это пока не обнаруживается — нет механизма периодической переинспекции минта уже открытой позиции. Это войдёт в задачу подключения живого потока событий (Dataplane), а не является полным покрытием сейчас. Buy/Sell/PoolCreated ingestion тоже ещё не реализован — им нужна конвертация в USD, источник цены SOL/USD ещё не выбран и не проверен.
