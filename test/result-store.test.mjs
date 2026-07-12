import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { RESULT_PATHS, createResultStore, sanitizeSubmission } from "../scripts/result-store.mjs";

function submission(overrides = {}) {
  const totalPerPath = 16;
  const completedCases = totalPerPath * RESULT_PATHS.length;
  return {
    schemaVersion: 1,
    browser: { name: "Safari", version: "26.5.1", userAgent: "must-not-persist" },
    capabilities: { secureContext: true, webTransport: true },
    run: {
      mode: "exhaustive",
      plannedCases: completedCases,
      completedCases,
      cancelled: false,
    },
    metrics: {
      constructor: completedCases,
      ready: completedCases,
      datagram: 0,
      datagramUnavailable: completedCases,
      bidirectionalStream: completedCases,
      unidirectionalStream: completedCases,
      streamEcho: completedCases,
      fullEcho: 0,
    },
    paths: RESULT_PATHS.map((path) => ({
      path,
      total: totalPerPath,
      constructor: totalPerPath,
      ready: totalPerPath,
      datagram: 0,
      datagramUnavailable: totalPerPath,
      bidirectionalStream: totalPerPath,
      unidirectionalStream: totalPerPath,
      effects: [],
      rawError: "must-not-persist",
    })),
    targetBase: "https://private.example.invalid:9446",
    ...overrides,
  };
}

test("stores one anonymous latest snapshot and only material changes", async () => {
  const directory = await mkdtemp(join(tmpdir(), "wt-result-store-"));
  const filePath = join(directory, "results.json");
  const times = [
    new Date("2026-07-11T10:00:00Z"),
    new Date("2026-07-11T11:00:00Z"),
    new Date("2026-07-11T12:00:00Z"),
  ];
  const store = createResultStore({ filePath, clock: () => times.shift() });

  try {
    await store.load();
    const first = await store.record(submission());
    assert.equal(first.changed, true);
    assert.equal(store.snapshot().latest.length, 1);
    assert.equal(store.snapshot().latest[0].browser.version, "26");
    assert.equal(store.snapshot().changes.length, 1);

    const repeated = await store.record(submission());
    assert.equal(repeated.changed, false, repeated.differences.join("; "));
    assert.equal(store.snapshot().latest[0].receivedAt, "2026-07-11T11:00:00.000Z");
    assert.equal(store.snapshot().changes.length, 1);

    const changedInput = submission();
    changedInput.metrics.ready = 0;
    changedInput.metrics.bidirectionalStream = 0;
    changedInput.metrics.unidirectionalStream = 0;
    changedInput.metrics.streamEcho = 0;
    changedInput.metrics.datagramUnavailable = 0;
    for (const path of changedInput.paths) {
      path.ready = 0;
      path.bidirectionalStream = 0;
      path.unidirectionalStream = 0;
      path.datagramUnavailable = 0;
    }
    const changed = await store.record(changedInput);
    assert.equal(changed.changed, true);
    assert.equal(store.snapshot().latest.length, 1);
    assert.equal(store.snapshot().changes.length, 2);
    assert.ok(store.snapshot().changes[0].changes.includes("Ready 128 -> 0"));

    const persisted = await readFile(filePath, "utf8");
    assert.doesNotMatch(persisted, /must-not-persist|private\.example|userAgent|rawError/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("requires complete exhaustive coverage of all known paths", () => {
  const selected = submission();
  selected.run.mode = "selected";
  assert.throws(() => sanitizeSubmission(selected, new Date().toISOString()), /exhaustive/);

  const incomplete = submission();
  incomplete.paths.pop();
  assert.throws(() => sanitizeSubmission(incomplete, new Date().toISOString()), /all response paths/);
});

test("records API-unavailable browsers without synthetic network cases", () => {
  const unavailable = submission({
    browser: { name: "Brave", version: "152.0.1" },
    capabilities: { secureContext: true, webTransport: false },
    run: { mode: "exhaustive", plannedCases: 128, completedCases: 0, cancelled: false },
    metrics: {
      constructor: 0,
      ready: 0,
      datagram: 0,
      datagramUnavailable: 0,
      bidirectionalStream: 0,
      unidirectionalStream: 0,
      streamEcho: 0,
      fullEcho: 0,
    },
    paths: RESULT_PATHS.map((path) => ({
      path,
      total: 0,
      constructor: 0,
      ready: 0,
      datagram: 0,
      datagramUnavailable: 0,
      bidirectionalStream: 0,
      unidirectionalStream: 0,
      effects: [],
    })),
  });

  const sanitized = sanitizeSubmission(unavailable, "2026-07-11T10:00:00.000Z");
  assert.equal(sanitized.browser.key, "brave");
  assert.equal(sanitized.conclusion, "WebTransport API unavailable");
  assert.equal(sanitized.paths[0].signal, "WebTransport API unavailable");
});
