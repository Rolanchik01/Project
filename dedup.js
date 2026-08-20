import { appendFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { validateEvent } from './domain.js';

/** Appends normalized events as NDJSON; the input recording stays immutable. */
export class NdjsonRecorder {
  constructor(filePath) {
    this.filePath = filePath;
    mkdirSync(dirname(filePath), { recursive: true });
  }

  record(event) {
    appendFileSync(this.filePath, `${JSON.stringify(validateEvent(event))}\n`, 'utf8');
  }
}
