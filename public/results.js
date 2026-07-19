const statusNode = document.querySelector("#status");
const refreshButton = document.querySelector("#refresh");

const OPTION_LABELS = new Map([
  ["allowPooling", "allowPooling=false"],
  ["requireUnreliable", "requireUnreliable=true"],
  ["congestionControl", "congestionControl=low-latency"],
  ["protocols", "protocols=quicast-wt-v0"],
  ["serverCertificateHashes", "serverCertificateHashes=sha-256"],
]);

refreshButton.addEventListener("click", loadResults);
loadResults();

async function loadResults() {
  refreshButton.disabled = true;
  statusNode.textContent = "Loading latest browser results";
  try {
    const response = await fetch("/api/browser-results", { cache: "no-store" });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error ?? `results request failed: ${response.status}`);
    render(body);
    statusNode.textContent = `Updated ${formatDate(body.generatedAt)}`;
  } catch (error) {
    statusNode.textContent = error.message ?? String(error);
  } finally {
    refreshButton.disabled = false;
  }
}

function render(data) {
  const latest = Array.isArray(data.latest) ? data.latest : [];
  const changes = Array.isArray(data.changes) ? data.changes : [];
  document.querySelector("#browserCount").textContent = String(latest.length);
  document.querySelector("#changeCount").textContent = String(changes.length);
  document.querySelector("#lastResult").textContent = latest.length > 0
    ? formatDate(latest.reduce((latestDate, item) =>
        Date.parse(item.receivedAt) > Date.parse(latestDate) ? item.receivedAt : latestDate,
      latest[0].receivedAt))
    : "None";
  renderLatest(latest);
  renderGrease(latest);
  renderPaths(latest);
  renderMatrices(latest);
  renderChanges(changes);
}

function renderLatest(latest) {
  const body = document.querySelector("#latestBody");
  body.replaceChildren();
  document.querySelector("#latestEmpty").hidden = latest.length > 0;
  document.querySelector("#latestWrap").hidden = latest.length === 0;

  for (const result of latest) {
    const row = document.createElement("tr");
    appendCell(row, browserLabel(result.browser));
    appendCell(row, formatDate(result.receivedAt));
    appendCell(row, result.capabilities.webTransport ? "available" : "unavailable",
      result.capabilities.webTransport ? "good" : "bad");
    appendCell(row, ratio(result.run.completedCases, result.run.plannedCases));
    appendMetric(row, result.metrics.constructor, result.run.completedCases);
    appendMetric(row, result.metrics.ready, result.run.completedCases);
    appendMetric(row, result.metrics.bidirectionalStream, result.run.completedCases);
    appendMetric(row, result.metrics.unidirectionalStream, result.run.completedCases);
    appendCell(row, datagramText(result), datagramClass(result));
    appendCell(row, result.conclusion, conclusionClass(result));
    body.appendChild(row);
  }
}

function renderGrease(latest) {
  const section = document.querySelector("#greaseSection");
  const body = document.querySelector("#greaseBody");
  const results = latest.filter((result) => result.grease);
  body.replaceChildren();
  section.hidden = results.length === 0;

  for (const result of results) {
    const row = document.createElement("tr");
    appendCell(row, browserLabel(result.browser));
    appendOutcome(row, result.grease.control.ready);
    appendOutcome(row, result.grease.enabled.ready);
    appendCell(
      row,
      greaseEchoText(result.grease.control),
      greaseEchoClass(result.grease.control),
    );
    appendCell(
      row,
      greaseEchoText(result.grease.enabled),
      greaseEchoClass(result.grease.enabled),
    );
    appendCell(
      row,
      greaseVerdictText(result.grease.verdict),
      greaseVerdictClass(result.grease.verdict),
    );
    body.appendChild(row);
  }
}

function greaseEchoText(probe) {
  if (probe?.ready !== "pass") return "not run";
  if (
    probe.datagram === "pass" &&
    probe.bidirectionalStream === "pass" &&
    probe.unidirectionalStream === "pass"
  ) {
    return "full echo";
  }
  if (
    probe.bidirectionalStream === "pass" &&
    probe.unidirectionalStream === "pass"
  ) {
    return probe.datagram === "unavailable" ? "stream echo" : "stream echo only";
  }
  return "failed";
}

function greaseEchoClass(probe) {
  const text = greaseEchoText(probe);
  if (text === "full echo") return "good";
  if (text === "not run") return "muted";
  if (text.startsWith("stream echo")) return "warn";
  return "bad";
}

function greaseVerdictText(verdict) {
  return {
    "api-unavailable": "API unavailable",
    "control-failed": "Control failed",
    affected: "Affected by GREASE",
    degraded: "Post-ready behavior changed",
    tolerant: "GREASE tolerated",
  }[verdict] ?? "Not tested";
}

function greaseVerdictClass(verdict) {
  if (verdict === "tolerant") return "good";
  if (verdict === "affected") return "bad";
  if (verdict === "degraded") return "warn";
  if (verdict === "control-failed") return "warn";
  return "muted";
}

function renderPaths(latest) {
  const section = document.querySelector("#pathSection");
  const root = document.querySelector("#pathDetails");
  root.replaceChildren();
  section.hidden = latest.length === 0;

  for (const result of latest) {
    const container = document.createElement("section");
    container.className = "browser-detail";
    const heading = document.createElement("div");
    heading.className = "detail-heading";
    const title = document.createElement("h3");
    title.textContent = browserLabel(result.browser);
    const received = document.createElement("span");
    received.textContent = formatDate(result.receivedAt);
    heading.append(title, received);

    const wrap = document.createElement("div");
    wrap.className = "table-wrap";
    const table = document.createElement("table");
    table.className = "path-table";
    table.appendChild(tableHead([
      "Path",
      "Constructor",
      "Ready",
      "Bidi",
      "Uni",
      "Datagram",
      "Option signal",
    ]));
    const body = document.createElement("tbody");
    for (const path of result.paths) {
      const row = document.createElement("tr");
      appendCell(row, path.path);
      appendMetric(row, path.constructor, path.total);
      appendMetric(row, path.ready, path.total);
      appendMetric(row, path.bidirectionalStream, path.total);
      appendMetric(row, path.unidirectionalStream, path.total);
      appendCell(
        row,
        path.datagramUnavailable === path.ready && path.ready > 0
          ? "not exposed"
          : ratio(path.datagram, path.total),
        path.datagramUnavailable === path.ready && path.ready > 0
          ? "warn"
          : metricClass(path.datagram, path.total),
      );
      appendCell(row, path.signal, path.ready > 0 ? "muted" : "bad");
      body.appendChild(row);
    }
    table.appendChild(body);
    wrap.appendChild(table);
    container.append(heading, wrap);
    root.appendChild(container);
  }
}

function renderMatrices(latest) {
  const section = document.querySelector("#matrixSection");
  const root = document.querySelector("#matrixDetails");
  const results = latest.filter((result) =>
    result.paths?.some((path) => Array.isArray(path.combinations) && path.combinations.length > 0),
  );
  root.replaceChildren();
  section.hidden = results.length === 0;

  for (const result of results) {
    const paths = result.paths.filter(
      (path) => Array.isArray(path.combinations) && path.combinations.length > 0,
    );
    const container = document.createElement("section");
    container.className = "browser-detail";

    const heading = document.createElement("div");
    heading.className = "detail-heading";
    const title = document.createElement("h3");
    title.textContent = browserLabel(result.browser);
    const received = document.createElement("span");
    received.textContent = formatDate(result.receivedAt);
    heading.append(title, received);

    const controls = document.createElement("div");
    controls.className = "matrix-controls";
    const label = document.createElement("label");
    label.textContent = "Response path";
    const select = document.createElement("select");
    for (const path of paths) {
      const option = document.createElement("option");
      option.value = path.path;
      option.textContent = path.path;
      select.appendChild(option);
    }
    label.appendChild(select);
    controls.appendChild(label);

    const wrap = document.createElement("div");
    wrap.className = "table-wrap";
    const table = document.createElement("table");
    table.className = "matrix-table";
    table.appendChild(tableHead([
      "Options",
      "Constructor",
      "Ready",
      "Bidi",
      "Uni",
      "Datagram",
    ]));
    const body = document.createElement("tbody");
    table.appendChild(body);
    wrap.appendChild(table);

    const renderSelectedPath = () => {
      const path = paths.find((item) => item.path === select.value) ?? paths[0];
      renderCombinationRows(body, path.combinations);
    };
    select.addEventListener("change", renderSelectedPath);
    renderSelectedPath();

    container.append(heading, controls, wrap);
    root.appendChild(container);
  }
}

function renderCombinationRows(body, combinations) {
  body.replaceChildren();
  for (const combination of combinations) {
    const row = document.createElement("tr");
    appendCell(row, optionText(combination.options));
    appendOutcome(row, combination.constructor);
    appendOutcome(row, combination.ready);
    appendOutcome(row, combination.bidirectionalStream);
    appendOutcome(row, combination.unidirectionalStream);
    appendOutcome(row, combination.datagram);
    body.appendChild(row);
  }
}

function optionText(options) {
  if (!Array.isArray(options) || options.length === 0) return "no options";
  return options.map((key) => OPTION_LABELS.get(key) ?? key).join(" + ");
}

function appendOutcome(row, outcome) {
  const labels = {
    pass: "pass",
    fail: "fail",
    unavailable: "not exposed",
    "not-run": "not run",
  };
  const classes = {
    pass: "good",
    fail: "bad",
    unavailable: "warn",
    "not-run": "muted",
  };
  appendCell(row, labels[outcome] ?? "unknown", classes[outcome] ?? "muted");
}

function renderChanges(changes) {
  const body = document.querySelector("#changesBody");
  body.replaceChildren();
  document.querySelector("#changesEmpty").hidden = changes.length > 0;
  document.querySelector("#changesWrap").hidden = changes.length === 0;
  for (const change of changes) {
    const row = document.createElement("tr");
    appendCell(row, formatDate(change.at));
    appendCell(row, browserLabel({ name: change.browserName, version: change.browserVersion }));
    appendCell(row, Array.isArray(change.changes) ? change.changes.join("; ") : "");
    body.appendChild(row);
  }
}

function tableHead(labels) {
  const head = document.createElement("thead");
  const row = document.createElement("tr");
  for (const label of labels) {
    const cell = document.createElement("th");
    cell.textContent = label;
    row.appendChild(cell);
  }
  head.appendChild(row);
  return head;
}

function appendMetric(row, value, total) {
  appendCell(row, ratio(value, total), metricClass(value, total));
}

function appendCell(row, value, className = "") {
  const cell = document.createElement("td");
  cell.textContent = String(value);
  cell.className = className;
  row.appendChild(cell);
}

function browserLabel(browser) {
  return browser.version ? `${browser.name} ${browser.version}` : browser.name;
}

function ratio(value, total) {
  return `${value}/${total}`;
}

function metricClass(value, total) {
  if (total === 0) return "muted";
  if (value === total) return "good";
  if (value > 0) return "warn";
  return "bad";
}

function datagramText(result) {
  if (
    result.metrics.streamEcho > 0 &&
    result.metrics.datagramUnavailable >= result.metrics.streamEcho
  ) {
    return "not exposed";
  }
  return ratio(result.metrics.datagram, result.run.completedCases);
}

function datagramClass(result) {
  return datagramText(result) === "not exposed"
    ? "warn"
    : metricClass(result.metrics.datagram, result.run.completedCases);
}

function conclusionClass(result) {
  if (!result.capabilities.webTransport || result.metrics.ready === 0) return "bad";
  if (result.metrics.fullEcho > 0) return "good";
  return "warn";
}

function formatDate(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "unknown";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}
