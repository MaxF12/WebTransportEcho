import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

export const RESULT_SCHEMA_VERSION = 1;
export const RESULT_PATHS = [
  "/wt/basic",
  "/wt/protocol",
  "/wt/capsule",
  "/wt/init",
  "/wt/draft",
  "/wt/h3-token",
  "/wt/yggdrasil",
  "/wt/auto",
];

const BROWSER_FAMILIES = new Map([
  ["brave", { key: "brave", name: "Brave" }],
  ["chrome", { key: "chrome", name: "Chrome" }],
  ["firefox", { key: "firefox", name: "Firefox" }],
  ["microsoft edge", { key: "edge", name: "Microsoft Edge" }],
  ["safari", { key: "safari", name: "Safari" }],
]);

const METRICS = [
  ["constructor", "Constructor"],
  ["ready", "Ready"],
  ["datagram", "Datagram"],
  ["datagramUnavailable", "Datagram unavailable"],
  ["bidirectionalStream", "Bidi"],
  ["unidirectionalStream", "Uni"],
  ["streamEcho", "Stream echo"],
  ["fullEcho", "Full echo"],
];

const PATH_METRICS = [
  ["constructor", "constructor"],
  ["ready", "ready"],
  ["bidirectionalStream", "bidi"],
  ["unidirectionalStream", "uni"],
  ["datagram", "datagram"],
  ["datagramUnavailable", "datagram unavailable"],
];

const OPTION_LABELS = new Map([
  ["allowPooling", "allowPooling=false"],
  ["requireUnreliable", "requireUnreliable=true"],
  ["congestionControl", "congestionControl=low-latency"],
  ["protocols", "protocols=quicast-wt-v0"],
  ["serverCertificateHashes", "serverCertificateHashes=sha-256"],
]);

export function createResultStore({ filePath, maxChanges = 100, clock = () => new Date() }) {
  let state = emptyState();
  let writeQueue = Promise.resolve();

  return {
    async load() {
      try {
        const parsed = JSON.parse(await readFile(filePath, "utf8"));
        if (
          parsed?.schemaVersion !== RESULT_SCHEMA_VERSION ||
          typeof parsed.latest !== "object" ||
          parsed.latest === null ||
          !Array.isArray(parsed.changes)
        ) {
          throw new Error("unsupported or malformed browser-results state");
        }
        state = {
          schemaVersion: RESULT_SCHEMA_VERSION,
          latest: parsed.latest,
          changes: parsed.changes.slice(-maxChanges),
        };
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
    },

    snapshot() {
      return publicSnapshot(state);
    },

    record(input) {
      const operation = writeQueue.catch(() => {}).then(async () => {
        const receivedAt = clock().toISOString();
        const current = sanitizeSubmission(input, receivedAt);
        const previous = state.latest[current.browser.key] ?? null;
        const differences = diffSnapshots(previous, current);
        let changes = [...state.changes];
        if (previous === null || differences.length > 0) {
          changes.push({
            at: receivedAt,
            browserKey: current.browser.key,
            browserName: current.browser.name,
            browserVersion: current.browser.version,
            changes: previous === null ? ["Initial result recorded"] : differences,
          });
          changes = changes.slice(-maxChanges);
        }

        const nextState = {
          schemaVersion: RESULT_SCHEMA_VERSION,
          latest: { ...state.latest, [current.browser.key]: current },
          changes,
        };
        await persist(filePath, nextState);
        state = nextState;
        return {
          browserKey: current.browser.key,
          changed: previous === null || differences.length > 0,
          differences,
          receivedAt,
        };
      });
      writeQueue = operation;
      return operation;
    },
  };
}

export function sanitizeSubmission(input, receivedAt) {
  if (input?.schemaVersion !== RESULT_SCHEMA_VERSION) {
    throw new TypeError("unsupported result schema");
  }

  const browser = normalizeBrowser(input.browser);
  const capabilities = {
    secureContext: requiredBoolean(input.capabilities?.secureContext, "secureContext"),
    webTransport: requiredBoolean(input.capabilities?.webTransport, "webTransport"),
  };
  const run = sanitizeRun(input.run, capabilities);
  const metrics = sanitizeMetrics(input.metrics, run.completedCases);
  const paths = sanitizePaths(input.paths, run, capabilities);

  return {
    receivedAt,
    browser,
    capabilities,
    run,
    metrics,
    conclusion: conclusionFor(capabilities, metrics),
    paths,
  };
}

export function browserKeyForSubmission(input) {
  return normalizeBrowser(input?.browser).key;
}

function emptyState() {
  return { schemaVersion: RESULT_SCHEMA_VERSION, latest: {}, changes: [] };
}

function normalizeBrowser(input) {
  const suppliedName = typeof input?.name === "string" ? input.name.toLowerCase() : "";
  const family = BROWSER_FAMILIES.get(suppliedName) ?? { key: "other", name: "Other" };
  const versionMatch = String(input?.version ?? "").match(/^(\d{1,4})/);
  return {
    ...family,
    version: versionMatch?.[1] ?? null,
  };
}

function sanitizeRun(input, capabilities) {
  if (input?.mode !== "exhaustive") {
    throw new TypeError("only exhaustive runs can be published");
  }
  if (input?.cancelled !== false) {
    throw new TypeError("cancelled runs cannot be published");
  }

  const plannedCases = requiredCount(input.plannedCases, "plannedCases");
  const completedCases = requiredCount(input.completedCases, "completedCases");
  if (plannedCases === 0 || completedCases > plannedCases) {
    throw new TypeError("invalid run coverage");
  }
  if (capabilities.webTransport && completedCases !== plannedCases) {
    throw new TypeError("incomplete runs cannot be published");
  }
  if (!capabilities.webTransport && completedCases !== 0) {
    throw new TypeError("API-unavailable runs cannot contain cases");
  }

  return { mode: "exhaustive", plannedCases, completedCases, cancelled: false };
}

function sanitizeMetrics(input, completedCases) {
  const metrics = {};
  for (const [key] of METRICS) {
    metrics[key] = requiredCount(input?.[key], key);
    if (metrics[key] > completedCases) {
      throw new TypeError(`${key} exceeds completedCases`);
    }
  }
  return metrics;
}

function sanitizePaths(input, run, capabilities) {
  if (!Array.isArray(input) || input.length !== RESULT_PATHS.length) {
    throw new TypeError("all response paths are required");
  }

  const supplied = new Map(input.map((item) => [item?.path, item]));
  if (supplied.size !== RESULT_PATHS.length) {
    throw new TypeError("response paths must be unique");
  }

  const paths = RESULT_PATHS.map((path) => {
    const item = supplied.get(path);
    if (!item) throw new TypeError(`missing response path ${path}`);
    const total = requiredCount(item.total, `${path}.total`);
    const sanitized = { path, total };
    for (const [key] of PATH_METRICS) {
      sanitized[key] = requiredCount(item[key], `${path}.${key}`);
      if (sanitized[key] > total) {
        throw new TypeError(`${path}.${key} exceeds total`);
      }
    }
    sanitized.effects = sanitizeEffects(item.effects, path);
    sanitized.signal = optionSignal(sanitized, capabilities);
    return sanitized;
  });

  const totalCases = paths.reduce((sum, item) => sum + item.total, 0);
  if (totalCases !== run.completedCases) {
    throw new TypeError("path totals do not match completedCases");
  }
  return paths;
}

function sanitizeEffects(input, path) {
  if (input === undefined) return [];
  if (!Array.isArray(input) || input.length > OPTION_LABELS.size) {
    throw new TypeError(`${path}.effects is invalid`);
  }
  const seen = new Set();
  return input.map((effect) => {
    if (!OPTION_LABELS.has(effect?.key) || seen.has(effect.key)) {
      throw new TypeError(`${path}.effects contains an invalid option`);
    }
    seen.add(effect.key);
    return {
      key: effect.key,
      withOption: sanitizeRatio(effect.withOption, `${path}.${effect.key}.withOption`),
      withoutOption: sanitizeRatio(effect.withoutOption, `${path}.${effect.key}.withoutOption`),
    };
  });
}

function sanitizeRatio(input, label) {
  const ready = requiredCount(input?.ready, `${label}.ready`);
  const total = requiredCount(input?.total, `${label}.total`);
  if (ready > total) throw new TypeError(`${label}.ready exceeds total`);
  return { ready, total };
}

function optionSignal(path, capabilities) {
  if (!capabilities.webTransport && path.total === 0) return "WebTransport API unavailable";
  if (path.ready === 0) return "No combination reached ready";
  if (path.ready === path.total) return "Every tested combination reached ready";

  const onlyWith = path.effects.filter(
    (effect) => effect.withOption.ready > 0 && effect.withoutOption.ready === 0,
  );
  const onlyWithout = path.effects.filter(
    (effect) => effect.withoutOption.ready > 0 && effect.withOption.ready === 0,
  );
  const signals = [];
  if (onlyWith.length > 0) {
    signals.push(`ready only with ${onlyWith.map((effect) => OPTION_LABELS.get(effect.key)).join(", ")}`);
  }
  if (onlyWithout.length > 0) {
    signals.push(`ready only without ${onlyWithout.map((effect) => OPTION_LABELS.get(effect.key)).join(", ")}`);
  }
  return signals.length > 0 ? signals.join("; ") : "Mixed or interacting option effects";
}

function conclusionFor(capabilities, metrics) {
  if (!capabilities.webTransport) return "WebTransport API unavailable";
  if (metrics.fullEcho > 0) return "Full datagram and stream echo works";
  if (metrics.streamEcho > 0 && metrics.datagramUnavailable >= metrics.streamEcho) {
    return "Ready and stream echo work; datagram API unavailable";
  }
  if (metrics.streamEcho > 0) return "Ready and stream echo work; datagram echo failed";
  if (metrics.ready > 0) return "Ready works; complete stream echo did not";
  if (metrics.constructor > 0) return "WebTransport exists; no case reached ready";
  return "All constructor calls failed";
}

function diffSnapshots(previous, current) {
  if (!previous) return [];
  const differences = [];
  if (previous.browser.version !== current.browser.version) {
    differences.push(`Version ${display(previous.browser.version)} -> ${display(current.browser.version)}`);
  }
  if (previous.capabilities.secureContext !== current.capabilities.secureContext) {
    differences.push(`Secure context ${previous.capabilities.secureContext} -> ${current.capabilities.secureContext}`);
  }
  if (previous.capabilities.webTransport !== current.capabilities.webTransport) {
    differences.push(`WebTransport API ${previous.capabilities.webTransport ? "available" : "unavailable"} -> ${current.capabilities.webTransport ? "available" : "unavailable"}`);
  }
  if (previous.run.plannedCases !== current.run.plannedCases) {
    differences.push(`Planned cases ${previous.run.plannedCases} -> ${current.run.plannedCases}`);
  }
  for (const [key, label] of METRICS) {
    if (previous.metrics[key] !== current.metrics[key]) {
      differences.push(`${label} ${previous.metrics[key]} -> ${current.metrics[key]}`);
    }
  }

  const previousPaths = new Map(previous.paths.map((item) => [item.path, item]));
  for (const path of current.paths) {
    const old = previousPaths.get(path.path);
    if (!old) {
      differences.push(`${path.path} added`);
      continue;
    }
    const pathDifferences = [];
    for (const [key, label] of PATH_METRICS) {
      if (old[key] !== path[key] || old.total !== path.total) {
        pathDifferences.push(`${label} ${old[key]}/${old.total} -> ${path[key]}/${path.total}`);
      }
    }
    if (old.signal !== path.signal) {
      pathDifferences.push("option signal changed");
    }
    if (pathDifferences.length > 0) {
      differences.push(`${path.path}: ${pathDifferences.join(", ")}`);
    }
  }
  return differences.slice(0, 64);
}

function publicSnapshot(state) {
  return {
    schemaVersion: RESULT_SCHEMA_VERSION,
    generatedAt: new Date().toISOString(),
    latest: Object.values(state.latest).sort((left, right) =>
      left.browser.name.localeCompare(right.browser.name),
    ),
    changes: [...state.changes].reverse(),
  };
}

async function persist(filePath, state) {
  await mkdir(dirname(filePath), { recursive: true });
  const temporary = `${filePath}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  await rename(temporary, filePath);
}

function requiredBoolean(value, label) {
  if (typeof value !== "boolean") throw new TypeError(`${label} must be boolean`);
  return value;
}

function requiredCount(value, label) {
  if (!Number.isSafeInteger(value) || value < 0 || value > 4096) {
    throw new TypeError(`${label} must be an integer between 0 and 4096`);
  }
  return value;
}

function display(value) {
  return value ?? "unknown";
}
