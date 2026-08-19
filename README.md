# Solana Momentum Recorder — Stage 0

Это **paper-only** основание для momentum-системы: единая модель событий, fail-closed контроль версии протокола, риск-снимок токена и детерминированный replay. В проекте нет загрузки ключей, RPC-вызовов и отправки транзакций.

## Что уже покрыто

- Нормализованные события: `TokenCreated`, `MetadataCreated`, `MintTo`, `AuthorityChanged`, `PoolCreated`, `CurveCreated`, `Buy`, `Sell`, `TokenTransfer`, `Graduation`, `Migration`, `LiquidityAdded`, `LiquidityRemoved`, а также `HolderSnapshot` и `NarrativeUpdated`.
- Снимок после каждого события: `safetyScore`, `creatorScore`, `demandScore`, концентрация держателей, давление продаж, ликвидность на выход и вероятность graduation.
- Жёсткие стоп-факторы: активные mint/freeze authority, transfer hook/fee и удаление ликвидности.
- Два входа: `confirmed_entry` для обычного сильного кандидата и уменьшенный `probe_entry` для сильного нарратива с неизвестным создателем.
- Контракт venue-адаптера с фиксированной версией layout/IDL и автоматическим halt при несовпадении версии.
- Replay сортирует события по slot, времени наблюдения, signature и instruction index, поэтому одинаковый набор данных даёт одинаковый результат.

## Запуск проверок

```powershell
& 'C:\Users\Rolan\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe' --test
```

## Границы Stage 0

Числа в скоринге — начальные исследовательские пороги, а не торговая рекомендация и не финальная стратегия. Их можно менять только через версионируемую конфигурацию после walk-forward бэктеста. Следующие этапы: реальный Geyser-recorder, отдельные Pump/PumpSwap-декодеры и сохранение сырых событий в NDJSON/Parquet.
