/**
 * The JS contract mirrors the intended Rust VenueAdapter hot-path trait.
 * A production adapter must pin its program layout/IDL version and supply
 * quote test vectors before the venue is enabled for paper or live routing.
 */
export class VenueAdapter {
  decode(_rawEvent) { throw new Error('decode(rawEvent) must be implemented'); }
  applyUpdate(_accountUpdate) { throw new Error('applyUpdate(accountUpdate) must be implemented'); }
  quoteBuy(_amountIn) { throw new Error('quoteBuy(amountIn) must be implemented'); }
  quoteSell(_tokenAmount) { throw new Error('quoteSell(tokenAmount) must be implemented'); }
  buildBuy(_request) { throw new Error('buildBuy(request) must be implemented'); }
  buildSell(_request) { throw new Error('buildSell(request) must be implemented'); }
  liquidityRisk() { throw new Error('liquidityRisk() must be implemented'); }
  protocolVersion() { throw new Error('protocolVersion() must be implemented'); }
}

export class AdapterVersionMismatch extends Error {
  constructor(venue, expected, received) {
    super(`HALT ${venue}: expected protocol version ${expected}, received ${received}`);
    this.name = 'AdapterVersionMismatch';
  }
}

export class AdapterRegistry {
  #versions = new Map();

  register(venue, version) {
    if (!venue || !version) throw new Error('venue and version are required');
    this.#versions.set(venue, version);
    return this;
  }

  assertCompatible(event) {
    const expected = this.#versions.get(event.venue);
    if (!expected) throw new AdapterVersionMismatch(event.venue, 'registered adapter', event.programVersion);
    if (expected !== event.programVersion) throw new AdapterVersionMismatch(event.venue, expected, event.programVersion);
  }
}
