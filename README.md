# Solana Momentum Bot

Адаптивный snipe-бот для ранних движений на мемкоинах на Solana (PumpSwap, Raydium, Meteora и др.). Активная разработка — на **Rust** (`crates/`); `src/`, `test/`, `package.json` в корне — это архивный JS-референс Этапа 0, оставленный как читаемая спецификация. Их логика перенесена в `crates/core` один в один, с тем же набором тестов, и дальнейшая разработка идёт только в Rust.

## Структура

- `crates/core` — портированная логика Этапа 0: схема событий (`domain`), риск-движок (`risk_engine`, `scoring_config`), контракт venue-адаптера с fail-closed halt при несовпадении версии (`adapter_contract`), детерминированный replay (`replay`), дедупликация двух независимых Geyser-потоков (`dedup`), запись событий в NDJSON (`recorder`).
- `docs/VENUE_ADAPTER.md` — контракт venue-адаптера (сейчас уже реализуется как настоящий Rust `trait`, не только описание).
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

## Запуск проверок

```bash
cargo test
```

## Границы

Числа в скоринге — начальные исследовательские пороги, а не торговая рекомендация. Их можно менять только через версионируемую конфигурацию после walk-forward бэктеста. Полный `VenueAdapter` trait (decode/apply_update/quote_buy/quote_sell/build_buy/build_sell) сознательно ещё не объявлен в коде — он появится вместе с первым настоящим адаптером (Pump), чтобы его сигнатуры отражали реальные потребности декодирования, а не предположения наперёд.
