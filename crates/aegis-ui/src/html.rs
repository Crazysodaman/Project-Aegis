//! Self-contained dashboard page. No external assets, no remote fetches.
//!
//! `__AEGIS_TOKEN_META__` is replaced with the session-token meta tag only when
//! mutations are enabled. `__AEGIS_MUTATIONS__` becomes `true` or `false`.

pub(crate) const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
__AEGIS_TOKEN_META__
<title>Aegis Local Dashboard</title>
<style>
:root {
  --primary: #9400D3;
  --accent: #00C2FF;
  --tertiary: #39FF14;
  --bg: #08060d;
  --panel: #120c1c;
  --panel-edge: #2a1c3f;
  --text: #e8e2f5;
  --muted: #8b81a3;
  --danger: #ff4d6d;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  background: radial-gradient(circle at 20% 0%, #1b0d2c 0%, var(--bg) 55%);
  color: var(--text);
  font-family: ui-monospace, "Cascadia Mono", "Segoe UI Mono", Menlo, Consolas, monospace;
  min-height: 100vh;
  padding: 24px;
}
header {
  display: flex; flex-wrap: wrap; align-items: baseline; gap: 16px;
  border-bottom: 1px solid var(--panel-edge); padding-bottom: 16px; margin-bottom: 24px;
}
h1 { margin: 0; font-size: 28px; letter-spacing: 0.18em; color: var(--primary); }
h2 { margin: 0 0 12px; font-size: 14px; letter-spacing: 0.16em; text-transform: uppercase; color: var(--accent); }
.sub { color: var(--muted); font-size: 12px; }
main { display: grid; gap: 20px; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); }
section {
  background: var(--panel); border: 1px solid var(--panel-edge); border-radius: 10px;
  padding: 18px; min-width: 0;
}
section.wide { grid-column: 1 / -1; }
dl { display: grid; grid-template-columns: max-content 1fr; gap: 8px 16px; margin: 0; }
dt { color: var(--muted); }
dd { margin: 0; overflow-wrap: anywhere; }
.badge {
  display: inline-block; padding: 2px 10px; border-radius: 999px;
  font-size: 12px; letter-spacing: 0.08em; border: 1px solid var(--panel-edge);
}
.badge.good { color: var(--tertiary); border-color: var(--tertiary); }
.badge.warn { color: var(--accent); border-color: var(--accent); }
.badge.danger { color: var(--danger); border-color: var(--danger); }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--panel-edge); vertical-align: top; }
th { color: var(--muted); font-weight: 600; text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; }
td.mono { font-size: 12px; overflow-wrap: anywhere; }
.good { color: var(--tertiary); }
.warn { color: var(--accent); }
.danger { color: var(--danger); }
.muted { color: var(--muted); }
ul { margin: 8px 0 0; padding-left: 20px; }
li { margin: 4px 0; overflow-wrap: anywhere; }
.btn {
  background: transparent; color: var(--accent); border: 1px solid var(--accent);
  border-radius: 6px; padding: 4px 8px; margin: 2px; cursor: pointer; font: inherit; font-size: 11px;
}
.btn:hover { background: rgba(0, 194, 255, 0.12); }
.btn.danger { color: var(--danger); border-color: var(--danger); }
.btn.danger:hover { background: rgba(255, 77, 109, 0.12); }
#stop-controls, #actions-note { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 12px; }
</style>
</head>
<body data-mutations="__AEGIS_MUTATIONS__">
<header>
  <h1>AEGIS</h1>
  <span class="sub">local dashboard &middot; health <span id="health" class="good">…</span></span>
  <span class="sub">generated <span id="generated">—</span></span>
</header>
<main>
  <section>
    <h2>Platform</h2>
    <dl>
      <dt>callsign</dt><dd id="callsign">—</dd>
      <dt>platform id</dt><dd id="platform-id">—</dd>
      <dt>authority</dt><dd id="authority">—</dd>
      <dt>zero-core</dt><dd><span id="zero-core" class="badge warn">…</span></dd>
      <dt>core count</dt><dd id="core-count">—</dd>
      <dt>STOP</dt><dd><span id="stop-state" class="badge warn">…</span></dd>
    </dl>
    <div id="stop-controls"></div>
  </section>

  <section>
    <h2>Diagnostics</h2>
    <dl>
      <dt>overall</dt><dd><span id="diagnostics-status" class="badge warn">…</span></dd>
    </dl>
    <ul id="reasons"><li class="muted">Loading…</li></ul>
  </section>

  <section class="wide">
    <h2>Core roster</h2>
    <table>
      <thead>
        <tr>
          <th>Core UUID</th><th>#</th><th>Name</th><th>Lifecycle</th>
          <th>Runtime</th><th>PID</th><th>Integrity</th><th>Actions</th>
        </tr>
      </thead>
      <tbody id="cores-body"></tbody>
    </table>
  </section>

  <section class="wide">
    <h2>Modules</h2>
    <dl><dt>registry</dt><dd id="modules-status">—</dd></dl>
    <ul id="modules"></ul>
  </section>
</main>
<script>
const mutations = document.body.dataset.mutations === "true";
const tokenMeta = document.querySelector('meta[name="aegis-session-token"]');
const sessionToken = tokenMeta ? tokenMeta.getAttribute("content") : "";
const reasonActions = new Set([
  "core.deactivate", "core.reactivate", "core.uninstall_preserve",
  "safe.stop.set", "safe.stop.clear"
]);

function setText(id, value) {
  const node = document.getElementById(id);
  if (!node) return;
  node.textContent = (value === undefined || value === null || value === "") ? "—" : String(value);
}
function clear(node) { while (node.firstChild) { node.removeChild(node.firstChild); } }
function addCell(row, text, className) {
  const cell = document.createElement("td");
  cell.textContent = (text === undefined || text === null || text === "") ? "—" : String(text);
  if (className) cell.className = className;
  row.appendChild(cell);
  return cell;
}
async function fetchJSON(path) {
  const response = await fetch(path, { cache: "no-store", headers: { "Accept": "application/json" } });
  if (!response.ok) throw new Error(path + " " + response.status);
  return response.json();
}

async function loadHealth() {
  try {
    const health = await fetchJSON("/api/health");
    setText("health", health.ok ? "online" : "degraded");
    setText("generated", health.generated_at);
  } catch (error) { setText("health", "offline"); }
}

async function loadStatus() {
  try {
    const status = await fetchJSON("/api/status");
    setText("callsign", status.callsign);
    setText("platform-id", status.platform_id);
    setText("authority", status.authority);
    setText("core-count", status.core_count);
    setText("generated", status.generated_at);
    const zero = document.getElementById("zero-core");
    zero.textContent = status.zero_core ? "ZERO-CORE" : "CORES PRESENT";
    zero.className = "badge " + (status.zero_core ? "warn" : "good");
    const stop = document.getElementById("stop-state");
    stop.textContent = status.stop_active ? "STOP ACTIVE" : "STOP INACTIVE";
    stop.className = "badge " + (status.stop_active ? "danger" : "good");
  } catch (error) { setText("callsign", "unavailable"); }
}

async function loadCores() {
  const body = document.getElementById("cores-body");
  clear(body);
  let cores = [];
  try { cores = await fetchJSON("/api/cores"); } catch (error) { cores = []; }
  if (!cores.length) {
    const row = document.createElement("tr");
    const cell = document.createElement("td");
    cell.colSpan = 8;
    cell.className = "muted";
    cell.textContent = "No registered Cores.";
    row.appendChild(cell);
    body.appendChild(row);
    return;
  }
  for (const core of cores) {
    const row = document.createElement("tr");
    addCell(row, core.core_uuid, "mono");
    addCell(row, core.core_number);
    addCell(row, core.display_name);
    addCell(row, core.lifecycle_state);
    const runtime = core.runtime || {};
    const heartbeat = runtime.heartbeat_fresh ? " • heartbeat fresh" : "";
    addCell(row, (runtime.state || "unknown") + heartbeat, runtime.heartbeat_fresh ? "good" : "muted");
    addCell(row, runtime.pid);
    const integrity = core.integrity || {};
    addCell(row, integrity.status || "unknown", integrity.status === "verified" ? "good" : "warn");
    const actions = document.createElement("td");
    if (mutations) {
      const add = (label, action, danger) => {
        const button = document.createElement("button");
        button.textContent = label;
        button.className = danger ? "btn danger" : "btn";
        button.addEventListener("click", () => performAction(action, core.core_uuid));
        actions.appendChild(button);
      };
      add("start", "core.start", false);
      add("stop", "core.stop", false);
      add("deactivate", "core.deactivate", false);
      add("reactivate", "core.reactivate", false);
      add("retire", "core.uninstall_preserve", true);
    } else {
      actions.textContent = "read-only";
      actions.className = "muted";
    }
    row.appendChild(actions);
    body.appendChild(row);
  }
}

async function loadModules() {
  const list = document.getElementById("modules");
  clear(list);
  try {
    const data = await fetchJSON("/api/modules");
    setText("modules-status", data.status);
    if (!data.modules || !data.modules.length) {
      const item = document.createElement("li");
      item.className = "muted";
      item.textContent = "No modules installed.";
      list.appendChild(item);
      return;
    }
    for (const module of data.modules) {
      const item = document.createElement("li");
      item.textContent = module.name + " " + module.version + " — " + module.lifecycle_state;
      list.appendChild(item);
    }
  } catch (error) {
    setText("modules-status", "unavailable");
    const item = document.createElement("li");
    item.className = "muted";
    item.textContent = "Module registry unavailable.";
    list.appendChild(item);
  }
}

async function loadDiagnostics() {
  const overall = document.getElementById("diagnostics-status");
  try {
    const report = await fetchJSON("/api/diagnostics");
    overall.textContent = report.status;
    overall.className = "badge " + (report.status === "healthy" ? "good" : report.status === "degraded" ? "warn" : "danger");
    const reasons = document.getElementById("reasons");
    clear(reasons);
    if (!report.reasons || !report.reasons.length) {
      const item = document.createElement("li");
      item.className = "muted";
      item.textContent = "No active reasons.";
      reasons.appendChild(item);
      return;
    }
    for (const reason of report.reasons) {
      const item = document.createElement("li");
      item.textContent = reason;
      reasons.appendChild(item);
    }
  } catch (error) {
    overall.textContent = "unavailable";
    overall.className = "badge danger";
  }
}

function setupStopControls() {
  const panel = document.getElementById("stop-controls");
  if (!mutations) {
    panel.textContent = "Mutation controls are disabled for this dashboard.";
    panel.className = "muted";
    return;
  }
  const engage = document.createElement("button");
  engage.textContent = "Engage STOP";
  engage.className = "btn danger";
  engage.addEventListener("click", () => performAction("safe.stop.set"));
  const clearButton = document.createElement("button");
  clearButton.textContent = "Clear STOP";
  clearButton.className = "btn";
  clearButton.addEventListener("click", () => performAction("safe.stop.clear"));
  panel.appendChild(engage);
  panel.appendChild(clearButton);
}

async function performAction(action, coreUuid) {
  let reason = "";
  if (reasonActions.has(action)) {
    const entered = window.prompt("Reason required for " + action + ":");
    if (!entered || !entered.trim()) {
      window.alert("A reason is required.");
      return;
    }
    reason = entered.trim();
  }
  const payload = { action: action };
  if (coreUuid) payload.core_uuid = coreUuid;
  if (reason) payload.reason = reason;
  try {
    const response = await fetch("/api/actions", {
      method: "POST",
      cache: "no-store",
      headers: {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "X-Aegis-Token": sessionToken
      },
      body: JSON.stringify(payload)
    });
    const data = await response.json().catch(() => ({ ok: false, error: "invalid response" }));
    if (!response.ok || !data.ok) {
      window.alert("Action failed: " + (data.error || response.status));
    }
  } catch (error) {
    window.alert("Action failed: network error");
  }
  await refresh();
}

function refresh() {
  return Promise.allSettled([loadHealth(), loadStatus(), loadCores(), loadModules(), loadDiagnostics()]);
}

setupStopControls();
refresh();
setInterval(() => { refresh(); }, 3000);
</script>
</body>
</html>
"#;
