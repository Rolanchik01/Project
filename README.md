# Solana Momentum Bot

Адаптивный snipe-бот для ранних движений на мемкоинах на Solana (PumpSwap, Raydium, Meteora и др.). Активная разработка — на **Rust** (`crates/`); `src/`, `test/`, `package.json` в корне — это архивный JS-референс Этапа 0, оставленный как читаемая спецификация. Их логика перенесена в `crates/core` один в один, с тем же набором тестов, и дальнейшая разработка идёт только в Rust.

## Структура

- `crates/core` — портированная логика Этапа 0: схема событий (`domain`), риск-движок (`risk_engine`, `scoring_config`), контракт venue-адаптера с fail-closed halt при несовпадении версии (`adapter_contract`), детерминированный replay (`replay`), дедупликация двух независимых Geyser-потоков (`dedup`), запись событий в NDJSON (`recorder`).
- `crates/pump` — Pump bonding curve: декодер `BondingCurve` и constant-product quote-математика, сверенные с реальными данными mainnet (см. `crates/pump/src/lib.rs` и `crates/pump/idl/pump.json`).
- `crates/pumpswap` — PumpSwap AMM: декодер `Pool` и constant-product quote-математика, тоже сверенные с реальными сделками (`crates/pumpswap/idl/pump_amm.json`).
- `crates/token2022` — проверка опасных расширений Token-2022 (transfer fee/hook, permanent delegate, non-transferable, default-frozen) поверх официального крейта `spl-token-2022`, без ручного разбора TLV.
- `crates/ingest` — склеивающий слой над `core`/`pump`/`pumpswap`/`token2022`: собирает decoded `Candidate` + результат инспекции минта в `core::domain::Event`, который уже понимает risk-engine.
- `crates/live` — живой источник событий (Dataplane): `logsSubscribe` (Pump и PumpSwap) и `accountSubscribe` (Pyth SOL/USD, динамически отслеживаемые bonding curve/pool), с переподключением/backoff. `bin/pipeline.rs` — единый процесс, склеивающий всё это через `crates/ingest` в `risk_engine`/`recorder`, проверен живьём на mainnet (см. «Границы» ниже); `bin/pump_listener.rs`/`bin/pumpswap_listener.rs` — меньшие демонстрации одного venue без риск-движка.
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

`crates/ingest` теперь также умеет Buy/Sell/PoolCreated (не только TokenCreated) для обоих venue, с конвертацией в USD (`crates/ingest/src/price.rs`) — модуль не получает цену сам, а принимает `sol_usd_price` от вызывающего кода. Ключевая находка: у PumpSwap-пулов SOL не всегда на одной и той же стороне — из ~20 реальных проверенных пулов примерно половина имеет SOL как `base_mint`, половина как `quote_mint`, один пул вообще без SOL (пара с USDC). Обе функции конвертации проверяют обе стороны и отказывают, если ни одна не SOL, вместо того чтобы предполагать фиксированную конвенцию.

Источник цены SOL/USD выбран и подключён (`crates/ingest/src/price_feed.rs`): Pyth Network отдаёт большинство `PriceUpdateV2`-аккаунтов как произвольные pull-oracle PDA, которые никто не обязан обновлять, но у небольшого набора «спонсируемых» фидов (включая SOL/USD) есть фиксированный адрес, который Pyth Data Association обновляет непрерывно — `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE` (owner `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`). Байтовый layout `PriceUpdateV2` сверен вручную с двумя реальными снимками mainnet, не просто взят из документации. Читается через уже существующий `accountSubscribe` — отдельного RPC-провайдера не потребовалось.

`crates/pumpswap` теперь декодирует и `DepositEvent`/`WithdrawEvent` (добавление/вывод ликвидности), а `crates/ingest` генерирует из них `LiquidityAdded`/`LiquidityRemoved`. Реальные данные показали, что `DepositEvent` существует в двух разных байтовых формах под одним дискриминатором: «полная» (248 байт, совпадает с IDL) и более короткая (105 байт, всегда в одной транзакции с `SellEvent`) — короткая форма сознательно не декодируется (`None`, fail-closed), а не угадывается наугад. Отдельная находка: PumpSwap навсегда блокирует ровно 100 «сырых» LP-единиц при создании пула (анти-манипуляционный паттерн в духе Uniswap V2, подтверждено прямым повторным декодированием реального `CreatePoolEvent.minimum_liquidity`) — поэтому `all_liquidity_removed` в `WithdrawEvent` проверяет `lp_mint_supply - lp_token_amount_in <= 100`, а не `== 0`; без этой поправки реальный вывод, почти полностью осушивший пул, не был бы распознан как полное изъятие ликвидности.

## Запуск проверок

```bash
cargo test --workspace
```

## Границы

Числа в скоринге — начальные исследовательские пороги, а не торговая рекомендация. Их можно менять только через версионируемую конфигурацию после walk-forward бэктеста. Комиссии Pump/PumpSwap не пересчитываются локально — только читаются из реальных исторических событий при replay (см. выше).

Безопасность минта (`TokenCreated`) проверяется один раз, в момент создания. Если авторитет комиссии/заморозки меняет параметры Token-2022 расширений уже после запуска (например, поднимает `transfer_fee_bps` или включает `DefaultAccountState = Frozen` постфактум), это пока не обнаруживается — нет механизма периодической переинспекции минта уже открытой позиции. Это войдёт в задачу подключения живого потока событий (Dataplane), а не является полным покрытием сейчас.

`crates/live` теперь имеет полноценный однопроцессный pipeline (`bin/pipeline.rs`) — `logsSubscribe` для Pump и PumpSwap плюс `accountSubscribe` для цены Pyth SOL/USD и динамически отслеживаемых bonding curve/pool, всё в одном процессе, со склейкой через `crates/ingest` в `risk_engine::apply_event` и запись в NDJSON (`recordings/pipeline.ndjson`). Проверено живьём на реальном mainnet: `TokenCreated`/`Buy`/`Sell`/`PoolCreated`/`LiquidityAdded`/`LiquidityRemoved`/`Graduation` — все наблюдались и оценивались риск-движком end-to-end. Слой, решающий, *какие именно* аккаунты смотреть (когда появляется новый bonding curve/pool из живых `TokenCreated`/`PoolCreated` событий), реализован в самом `pipeline.rs`.

Безопасность минта (Token-2022 флаги) читается не через `accountSubscribe`, а одноразовым HTTPS `getAccountInfo` (`crates/live/src/rpc_fetch.rs`) сразу после `TokenCreated` — живая проверка показала, что `accountSubscribe` не отдаёт начальный снимок состояния аккаунта, только уведомления о последующих изменениях, а у свежесозданного минта последняя on-chain запись часто и есть сама транзакция создания, так что подписка на него может вообще не дать уведомлений за всё время жизни процесса. Отдельная живая находка: у публичного multi-node RPC-эндпоинта HTTP-запрос и WebSocket-подписка могут обслуживаться разными бэкендами не в фазе — `getAccountInfo`, отправленный сразу после `TokenCreated`, может на мгновение вернуть «аккаунт не найден», хотя аккаунт уже существует; `fetch_account_with_retry` делает до 3 попыток именно на этот случай (наблюдалось живьём: 1 из 18 минтов в тестовом прогоне, снялось следующей же попыткой).

`PoolCreated`/`exit_liquidity_usd` (PumpSwap) — по-прежнему снимок ликвидности на момент создания пула; `LiquidityAdded`/`LiquidityRemoved` теперь генерируются в реальном времени при живых Deposit/Withdraw-событиях того же пула, но сам `exit_liquidity_usd` не пересчитывается задним числом от них — это отдельная задача агрегации состояния пула поверх потока событий, не текущего Stage 1.

Известные упрощения `pipeline.rs`, не исправленные сейчас: Token-2022 флаги минта проверяются только один раз (при создании, тот же пробел, что описан выше); ни один отслеживаемый аккаунт никогда не снимается с подписки (`Unwatch` не используется) — нормально для исследовательского прогона, не для долгоживущего процесса с тысячами токенов; один процесс, одно соединение на подписку, нет резервного RPC-провайдера (Geyser/Helius) на случай проблем с публичным RPC.
