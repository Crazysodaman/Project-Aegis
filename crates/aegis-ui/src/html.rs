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
a.btn { text-decoration: none; display: inline-block; }
a { color: var(--accent); }
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
  <span class="sub"><a href="/api/status">platform status API</a></span>
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

  __AEGIS_STATUS_SECTIONS__

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
    const workspace = document.createElement("a");
    workspace.textContent = "workspace";
    workspace.href = "/core/" + encodeURIComponent(core.core_uuid);
    workspace.className = "btn";
    actions.appendChild(workspace);
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
      const note = document.createElement("span");
      note.textContent = " read-only";
      note.className = "muted";
      actions.appendChild(note);
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

use crate::gui::{CoreWorkspaceView, SectionStatus, WorkspaceSection};

const WORKSPACE_CSS: &str = r#"
:root {
  --primary: #9400D3;
  --accent: #00C2FF;
  --tertiary: #39FF14;
  --bg: #000000;
  --panel: #120c1c;
  --panel-edge: #2a1c3f;
  --text: #e8e2f5;
  --muted: #8b81a3;
  --danger: #ff4d6d;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body.core-theme {
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
h1 { margin: 0; font-size: 28px; letter-spacing: 0.14em; color: var(--primary); }
h2 { margin: 0 0 12px; font-size: 14px; letter-spacing: 0.16em; text-transform: uppercase; color: var(--accent); }
.sub { color: var(--muted); font-size: 12px; }
.crumbs { color: var(--muted); font-size: 12px; display: flex; gap: 6px; align-items: baseline; margin-left: auto; }
.crumbs a { color: var(--accent); text-decoration: none; }
.crumbs a:hover { color: var(--tertiary); }
.crumbs .sep { color: var(--muted); }
.header-links { display: flex; gap: 12px; align-items: baseline; }
.back { color: var(--accent); font-size: 12px; text-decoration: none; }
.back:hover { color: var(--tertiary); }
.core-grid { display: grid; gap: 20px; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); }
.core-section {
  background: var(--panel); border: 1px solid var(--panel-edge); border-radius: 10px;
  padding: 18px; min-width: 0;
}
.core-section[data-status="unavailable"] { border-style: dashed; }
.note { color: var(--muted); font-size: 12px; margin: 0 0 12px; overflow-wrap: anywhere; }
dl { display: grid; grid-template-columns: max-content 1fr; gap: 8px 16px; margin: 0; }
dt { color: var(--muted); }
dd { margin: 0; overflow-wrap: anywhere; }
ul { margin: 8px 0 0; padding-left: 20px; }
li { margin: 4px 0; overflow-wrap: anywhere; }
.badge {
  display: inline-block; padding: 2px 10px; border-radius: 999px;
  font-size: 11px; letter-spacing: 0.08em; border: 1px solid var(--panel-edge);
}
.badge.good { color: var(--tertiary); border-color: var(--tertiary); }
.badge.warn { color: var(--accent); border-color: var(--accent); }
.badge.danger { color: var(--danger); border-color: var(--danger); }
footer { color: var(--muted); font-size: 12px; margin-top: 24px; }
a { color: var(--accent); }
"#;

pub(crate) fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn status_class(status: SectionStatus) -> &'static str {
    match status {
        SectionStatus::Available => "good",
        SectionStatus::Partial | SectionStatus::Stale => "warn",
        SectionStatus::Unavailable => "danger",
    }
}

fn status_label(status: SectionStatus) -> &'static str {
    match status {
        SectionStatus::Available => "AVAILABLE",
        SectionStatus::Partial => "PARTIAL",
        SectionStatus::Stale => "STALE",
        SectionStatus::Unavailable => "UNAVAILABLE",
    }
}

fn status_value(status: SectionStatus) -> &'static str {
    match status {
        SectionStatus::Available => "available",
        SectionStatus::Partial => "partial",
        SectionStatus::Stale => "stale",
        SectionStatus::Unavailable => "unavailable",
    }
}

fn render_section(section: &WorkspaceSection, out: &mut String) {
    out.push_str("<section class=\"core-section\" data-status=\"");
    out.push_str(status_value(section.status));
    out.push_str("\">\n<h2>");
    out.push_str(&escape_html(section.title));
    out.push_str(" <span class=\"badge ");
    out.push_str(status_class(section.status));
    out.push_str("\">");
    out.push_str(status_label(section.status));
    out.push_str("</span></h2>\n");
    if let Some(note) = &section.note {
        out.push_str("<p class=\"note\">");
        out.push_str(&escape_html(note));
        out.push_str("</p>\n");
    }
    if !section.rows.is_empty() {
        out.push_str("<dl>\n");
        for (label, value) in &section.rows {
            out.push_str("<dt>");
            out.push_str(&escape_html(label));
            out.push_str("</dt><dd>");
            out.push_str(&escape_html(value));
            out.push_str("</dd>\n");
        }
        out.push_str("</dl>\n");
    }
    if !section.items.is_empty() {
        out.push_str("<ul>\n");
        for item in &section.items {
            out.push_str("<li>");
            out.push_str(&escape_html(item));
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
}

pub(crate) fn render_sections(sections: &[WorkspaceSection]) -> String {
    let mut out = String::with_capacity(1024 * sections.len());
    for section in sections {
        render_section(section, &mut out);
    }
    out
}

pub(crate) fn render_core_workspace(view: &CoreWorkspaceView) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
    );
    out.push_str(&escape_html(&format!(
        "{} - Aegis Core Workspace",
        view.display_name
    )));
    out.push_str("</title>\n<style>\n");
    out.push_str(WORKSPACE_CSS);
    out.push_str("</style>\n</head>\n<body class=\"core-theme\" data-core-uuid=\"");
    out.push_str(&escape_html(&view.core_uuid));
    out.push_str("\">\n<header>\n<h1>");
    out.push_str(&escape_html(&view.display_name));
    out.push_str("</h1>\n<span class=\"sub\">Core ");
    out.push_str(&escape_html(&view.core_uuid));
    out.push_str(" &middot; #");
    out.push_str(&escape_html(&view.core_number.to_string()));
    out.push_str(" &middot; ");
    out.push_str(&escape_html(&view.lifecycle_state));
    out.push_str("</span>\n<span class=\"sub\">generated ");
    out.push_str(&escape_html(&view.generated_at));
    out.push_str("</span>\n<nav class=\"crumbs\" aria-label=\"Breadcrumb\"><a href=\"/\">Dashboard</a><span class=\"sep\">/</span><span>Core ");
    out.push_str(&escape_html(&view.core_uuid));
    out.push_str("</span></nav>\n<span class=\"header-links\"><a class=\"back\" href=\"/api/core/");
    out.push_str(&escape_html(&view.core_uuid));
    out.push_str(
        "\">JSON API</a><a class=\"back\" href=\"/api/status\">platform status</a><a class=\"back\" href=\"/\">&larr; dashboard</a></span>\n</header>\n",
    );
    out.push_str("<main class=\"core-grid\">\n");
    for section in &view.sections {
        render_section(section, &mut out);
    }
    out.push_str("</main>\n<footer>HTTP loopback dashboard (not a native window). ");
    out.push_str("Wired sections are read from the local Aegis backend; sections marked ");
    out.push_str("UNAVAILABLE have no linked data source.</footer>\n</body>\n</html>\n");
    out
}

pub(crate) fn render_core_workspace_error(title: &str, message: &str) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>Aegis Core Workspace Error</title>\n<style>\n");
    out.push_str(WORKSPACE_CSS);
    out.push_str("</style>\n</head>\n<body class=\"core-theme\">\n<header>\n<h1>AEGIS</h1>\n");
    out.push_str("<a class=\"back\" href=\"/\">&larr; dashboard</a>\n</header>\n<main>\n");
    out.push_str("<section class=\"core-section\" data-status=\"unavailable\">\n<h2>");
    out.push_str(&escape_html(title));
    out.push_str("</h2>\n<p class=\"note\">");
    out.push_str(&escape_html(message));
    out.push_str("</p>\n</section>\n</main>\n</body>\n</html>\n");
    out
}
