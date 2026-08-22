import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { NdjsonRecorder } from '../src/recorder.js';
import { confirmedCandidateEvents } from './helpers.js';

test('NdjsonRecorder appends validated events as newline-delimited JSON, creating parent directories', () => {
  const dir = mkdtempSync(join(tmpdir(), 'recorder-test-'));
  const filePath = join(dir, 'nested', 'events.ndjson');
  try {
    const recorder = new NdjsonRecorder(filePath);
    const [tokenEvent, poolEvent] = confirmedCandidateEvents();
    recorder.record(tokenEvent);
    recorder.record(poolEvent);

    const lines = readFileSync(filePath, 'utf8').trim().split('\n');
    assert.equal(lines.length, 2);
    assert.deepEqual(JSON.parse(lines[0]), tokenEvent);
    assert.deepEqual(JSON.parse(lines[1]), poolEvent);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('NdjsonRecorder is fail-closed: it rejects an event that fails schema validation and writes nothing', () => {
  const dir = mkdtempSync(join(tmpdir(), 'recorder-test-'));
  const filePath = join(dir, 'events.ndjson');
  try {
    const recorder = new NdjsonRecorder(filePath);
    assert.throws(() => recorder.record({ kind: 'TokenCreated' }), /missing/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
