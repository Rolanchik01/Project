# Solana Momentum Bot

Адаптивный snipe-бот для ранних движений на мемкоинах на Solana (PumpSwap, Raydium, Meteora и др.). Активная разработка — на **Rust** (`crates/`); `src/`, `test/`, `package.json` в корне — это архивный JS-референс Этапа 0, оставленный как читаемая спецификация. Их логика перенесена в `crates/core` один в один, с тем же набором тестов, и дальнейшая разработка идёт только в Rust.

## Структура

- `crates/core` — портированная логика Этапа 0: схема событий (`domain`), риск-движок (`risk_engine`, `scoring_config`), контракт venue-адаптера с fail-closed halt при несовпадении версии (`adapter_contract`), детерминированный replay (`replay`), дедупликация двух независимых Geyser-потоков (`dedup`), запись событий в NDJSON (`recorder`).
- `crates/pump` — Pump bonding curve: декодер `BondingCurve` и constant-product quote-математика, сверенные с реальными данными mainnet (см. `crates/pump/src/lib.rs` и `crates/pump/idl/pump.json`).
- `crates/pumpswap` — PumpSwap AMM: декодер `Pool` и constant-product quote-математика, тоже сверенные с реальными сделками (`crates/pumpswap/idl/pump_amm.json`).
- `crates/token2022` — проверка опасных расширений Token-2022 (transfer fee/hook, permanent delegate, non-transferable, default-frozen) поверх официального крейта `spl-token-2022`, без ручного разбора TLV.
- `crates/ingest` — склеивающий слой над `core`/`pump`/`pumpswap`/`token2022`: собирает decoded `Candidate` + результат инспекции минта в `core::domain::Event`, который уже понимает risk-engine.
- `crates/live` — первый живой источник событий (Dataplane): WebSocket-подписка на `logsSubscribe` через публичный Solana RPC, с переподключением/backoff. Первый асинхронный/сетевой код в проекте (`crates/live/src/bin/pump_listener.rs` — рабочая демонстрация, реально запускалась против mainnet).
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

`crates/ingest` теперь также умеет Buy/Sell/PoolCreated (не только TokenCreated) для обоих venue, с конвертацией в USD (`crates/ingest/src/price.rs`) — модуль не получает цену сам, а принимает `sol_usd_price` от вызывающего кода (источник цены ещё не выбран). Ключевая находка: у PumpSwap-пулов SOL не всегда на одной и той же стороне — из ~20 реальных проверенных пулов примерно половина имеет SOL как `base_mint`, половина как `quote_mint`, один пул вообще без SOL (пара с USDC). Обе функции конвертации проверяют обе стороны и отказывают, если ни одна не SOL, вместо того чтобы предполагать фиксированную конвенцию.

## Запуск проверок

```bash
cargo test --workspace
```

## Границы

Числа в скоринге — начальные исследовательские пороги, а не торговая рекомендация. Их можно менять только через версионируемую конфигурацию после walk-forward бэктеста. Комиссии Pump/PumpSwap не пересчитываются локально — только читаются из реальных исторических событий при replay (см. выше).

Безопасность минта (`TokenCreated`) проверяется один раз, в момент создания. Если авторитет комиссии/заморозки меняет параметры Token-2022 расширений уже после запуска (например, поднимает `transfer_fee_bps` или включает `DefaultAccountState = Frozen` постфактум), это пока не обнаруживается — нет механизма периодической переинспекции минта уже открытой позиции. Это войдёт в задачу подключения живого потока событий (Dataplane), а не является полным покрытием сейчас.

`PoolCreated`/`exit_liquidity_usd` (PumpSwap) — это снимок ликвидности только на момент создания пула. `DepositEvent`/`WithdrawEvent` (добавление/вывод ликвидности) ещё не декодируются в `crates/pumpswap`, поэтому `LiquidityAdded`/`LiquidityRemoved` не генерируются — цифра устаревает сразу после создания пула, пока эти декодеры не появятся. Источник цены SOL/USD для `sol_usd_price` тоже ещё не выбран и не подключён — `crates/ingest` принимает его как параметр, не вычисляет сам.

`crates/live` пока умеет только `logsSubscribe` для Pump и печатает декодированные события — не подключён к `crates/ingest`/risk-engine/`recorder` (эта склейка ещё впереди), нет `accountSubscribe` для reserve-аккаунтов/минтов (нужен для `apply_update`/USD-конвертации в реальном времени), нет второй копии для PumpSwap (тот же паттерн, просто не продублирован), нет резервного провайдера (Geyser/Helius) на случай проблем с публичным RPC.
