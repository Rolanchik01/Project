# Контракт protocol adapter

Hot-path реализация будет на Rust. Пока recorder/replay работают в portable JavaScript, этот контракт — неизменяемая граница для Pump, PumpSwap, Raydium и Meteora.

```rust
trait VenueAdapter {
    fn decode(&self, event: &RawEvent) -> Option<Candidate>;
    fn apply_update(&mut self, update: &AccountUpdate) -> Result<()>;

    fn quote_buy(&self, amount_in: u64) -> Result<Quote>;
    fn quote_sell(&self, token_amount: u64) -> Result<Quote>;

    fn build_buy(&self, request: &TradeRequest) -> Result<VersionedTransaction>;
    fn build_sell(&self, request: &TradeRequest) -> Result<VersionedTransaction>;

    fn liquidity_risk(&self) -> LiquidityRisk;
    fn protocol_version(&self) -> ProtocolVersion;
}
```

Перед включением любого адаптера обязательны:

1. Фиксированная версия program layout/IDL и тест с реальным account data.
2. Набор quote-векторов: вход, выход, price impact, fees, недостаточная ликвидность.
3. Replay-совместимость с записанными mainnet событиями.
4. Fail-closed: неизвестная версия, неверная длина аккаунта или ошибка декодера переводит venue в `HALT`; он не может строить заявки до явного обновления и прохождения тестов.

Program IDs не зашиты в код Stage 0. Их нужно вносить в отдельный подписанный registry только после проверки по актуальной официальной документации и IDL конкретной программы.
