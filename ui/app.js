/* VMI Analysis Client control panel.
 *
 * Runs in the system browser and talks to the local client service over
 * loopback HTTP. The per-run token from the address bar authorizes every
 * request and the event stream.
 */

const token = new URLSearchParams(window.location.search).get("token") || "";

const el = (id) => document.getElementById(id);

const ui = {
  form: el("connection-form"),
  name: el("name"),
  host: el("host"),
  port: el("port"),
  user: el("user"),
  authKind: el("auth-kind"),
  password: el("password"),
  passwordField: el("password-field"),
  keyField: el("key-field"),
  keyPath: el("key-path"),
  passphrase: el("passphrase"),
  passphraseField: el("passphrase-field"),
  remember: el("remember"),
  remoteHost: el("remote-host"),
  remotePort: el("remote-port"),
  localPort: el("local-port"),
  autoOpen: el("auto-open"),
  profileSelect: el("profile-select"),
  connect: el("connect-button"),
  disconnect: el("disconnect-button"),
  save: el("save-button"),
  remove: el("delete-button"),
  open: el("open-button"),
  copy: el("copy-button"),
  quit: el("quit-button"),
  error: el("form-error"),
  pill: el("status-pill"),
  pillLabel: el("status-label"),
  message: el("state-message"),
  url: el("local-url"),
  log: el("log"),
  clearLog: el("clear-log"),
  version: el("version"),
  service: el("service-note"),
};

let profiles = [];
let status = { state: "disconnected", message: "Not connected.", url: null };
let autoOpenedUrl = null;

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...(options.headers || {}),
    },
  });
  if (!response.ok) {
    const text = (await response.text()).trim();
    throw new Error(text || `Request failed (${response.status})`);
  }
  const type = response.headers.get("content-type") || "";
  return type.includes("application/json") ? response.json() : null;
}

function showError(message) {
  if (!message) {
    ui.error.hidden = true;
    return;
  }
  ui.error.textContent = message;
  ui.error.hidden = false;
}

function setBusy(busy) {
  ui.connect.disabled =
    busy ||
    status.state === "connected" ||
    status.state === "connecting" ||
    status.state === "reconnecting";
  ui.connect.textContent = busy ? "Connecting…" : "Connect";
}

function logLine(level, message) {
  const item = document.createElement("li");
  const time = document.createElement("time");
  const label = document.createElement("span");
  const text = document.createElement("span");

  time.textContent = new Date().toLocaleTimeString([], { hour12: false });
  label.textContent = level;
  label.className = `level level-${level}`;
  text.textContent = message;

  item.append(time, label, text);
  ui.log.prepend(item);
  while (ui.log.children.length > 250) {
    ui.log.lastElementChild.remove();
  }
}

function applyStatus(next) {
  status = { ...status, ...next };
  const state = status.state || "disconnected";

  ui.pill.className = `pill pill-${state}`;
  ui.pillLabel.textContent = state.charAt(0).toUpperCase() + state.slice(1);
  ui.message.textContent = status.message || "";

  const connected = state === "connected";
  const busyState = state === "connecting" || state === "reconnecting";

  ui.url.textContent = status.url || "No local address yet";
  ui.url.classList.toggle("active", connected);
  ui.open.disabled = !connected;
  ui.copy.disabled = !status.url;
  ui.disconnect.disabled = !(connected || busyState);
  ui.connect.disabled = connected || busyState;
  ui.connect.textContent = "Connect";

  if (connected && status.url && ui.autoOpen.checked && autoOpenedUrl !== status.url) {
    autoOpenedUrl = status.url;
    api("/api/open", {
      method: "POST",
      body: JSON.stringify({ url: status.url }),
    }).catch((err) => logLine("error", err.message));
  }
  if (state === "failed") {
    showError(status.message);
  }
}

function fillForm(profile) {
  ui.name.value = profile.name || "";
  ui.host.value = profile.host || "";
  ui.port.value = profile.port || 22;
  ui.user.value = profile.user || "";
  ui.authKind.value = profile.auth && profile.auth.kind === "key_file" ? "key_file" : "password";
  ui.keyPath.value =
    profile.auth && profile.auth.kind === "key_file" ? profile.auth.path || "" : "";
  ui.remoteHost.value = profile.remote_host || "127.0.0.1";
  ui.remotePort.value = profile.remote_port || 13517;
  ui.localPort.value = profile.local_port ? profile.local_port : "";
  ui.remember.checked = Boolean(profile.remember_password || profile.remember_passphrase);
  ui.password.value = "";
  ui.passphrase.value = "";
  syncAuthFields();
  showError("");
}

function syncAuthFields() {
  const keyMode = ui.authKind.value === "key_file";
  ui.passwordField.hidden = keyMode;
  ui.keyField.hidden = !keyMode;
  ui.passphraseField.hidden = !keyMode;
}

function readForm() {
  const keyMode = ui.authKind.value === "key_file";
  const localPortText = ui.localPort.value.trim();
  return {
    name: ui.name.value.trim(),
    host: ui.host.value.trim(),
    port: Number(ui.port.value) || 22,
    user: ui.user.value.trim(),
    auth: keyMode
      ? { kind: "key_file", path: ui.keyPath.value.trim() }
      : { kind: "password" },
    remote_host: ui.remoteHost.value.trim() || "127.0.0.1",
    remote_port: Number(ui.remotePort.value) || 13517,
    local_port: localPortText ? Number(localPortText) : 0,
    remember_password: ui.remember.checked,
    remember_passphrase: ui.remember.checked,
  };
}

function refreshProfiles(store) {
  profiles = (store && store.profiles) || [];
  const selected = ui.profileSelect.value;
  ui.profileSelect.replaceChildren();
  ui.profileSelect.append(new Option("New profile", ""));
  for (const profile of profiles) {
    const label = profile.name || `${profile.user}@${profile.host}`;
    ui.profileSelect.append(new Option(label, label));
  }
  if (profiles.some((profile) => (profile.name || "") === selected)) {
    ui.profileSelect.value = selected;
  }
  ui.remove.disabled = !ui.profileSelect.value;
}

function selectedProfile() {
  const name = ui.profileSelect.value;
  return profiles.find((profile) => (profile.name || "") === name) || null;
}

async function submit(event) {
  if (event) {
    event.preventDefault();
  }
  showError("");
  const profile = readForm();
  const keyMode = ui.authKind.value === "key_file";
  setBusy(true);
  try {
    const info = await api("/api/connect", {
      method: "POST",
      body: JSON.stringify({
        profile,
        password: keyMode ? null : ui.password.value,
        passphrase: keyMode ? ui.passphrase.value : null,
      }),
    });
    ui.password.value = "";
    ui.passphrase.value = "";
    applyStatus({ state: "connecting", message: "Connecting…", url: info.url });
    logLine("info", `Tunnel requested at ${info.url}`);
  } catch (err) {
    showError(err.message);
    logLine("error", err.message);
  } finally {
    setBusy(false);
  }
}

async function saveProfile() {
  showError("");
  try {
    const profile = readForm();
    const store = await api("/api/profiles", {
      method: "POST",
      body: JSON.stringify(profile),
    });
    refreshProfiles(store);
    ui.profileSelect.value = profile.name || `${profile.user}@${profile.host}`;
    ui.remove.disabled = false;
    logLine("info", "Profile saved.");
  } catch (err) {
    showError(err.message);
  }
}

async function deleteProfile() {
  const name = ui.profileSelect.value;
  if (!name) {
    return;
  }
  try {
    const store = await api(`/api/profiles/${encodeURIComponent(name)}`, { method: "DELETE" });
    refreshProfiles(store);
    fillForm({});
    ui.profileSelect.value = "";
    ui.remove.disabled = true;
    logLine("info", `Profile "${name}" deleted.`);
  } catch (err) {
    showError(err.message);
  }
}

async function disconnect() {
  try {
    const next = await api("/api/disconnect", { method: "POST", body: "{}" });
    applyStatus(next);
    logLine("info", "Disconnected by user.");
  } catch (err) {
    showError(err.message);
  }
}

async function openTunnel() {
  if (!status.url) {
    return;
  }
  try {
    await api("/api/open", { method: "POST", body: JSON.stringify({ url: status.url }) });
  } catch (err) {
    logLine("error", err.message);
  }
}

async function copyUrl() {
  if (!status.url) {
    return;
  }
  try {
    await navigator.clipboard.writeText(status.url);
    logLine("info", "Local address copied.");
  } catch {
    logLine("warn", "Could not access the clipboard.");
  }
}

async function browseKey() {
  try {
    const path = await api("/api/pick-key", { method: "POST", body: "{}" });
    if (path) {
      ui.keyPath.value = path;
    }
  } catch (err) {
    logLine("warn", err.message);
  }
}

async function quitClient() {
  try {
    await api("/api/quit", { method: "POST", body: "{}" });
    ui.service.textContent = "The client is stopping. This tab can be closed.";
    ui.pill.className = "pill pill-disconnected";
    ui.pillLabel.textContent = "Stopped";
  } catch (err) {
    logLine("error", err.message);
  }
}

function handleStatusEvent(payload) {
  applyStatus(payload);
}

function handleLogEvent(payload) {
  logLine(payload.level || "info", payload.message || "");
}

async function init() {
  if (!token) {
    showError("This panel needs the token from the client. Start the client again to reopen it.");
    return;
  }

  try {
    const store = await api("/api/profiles");
    refreshProfiles(store);
    const initial =
      profiles.find((profile) => (profile.name || "") === (store.last_used || "")) || profiles[0];
    if (initial) {
      ui.profileSelect.value = initial.name || "";
      fillForm(initial);
      ui.remove.disabled = false;
    }
    applyStatus(await api("/api/status"));
  } catch (err) {
    showError(err.message);
  }

  // The snapshot flag is used by the screenshot check; skip the live stream so
  // the page reaches an idle state.
  if (!new URLSearchParams(window.location.search).has("snapshot")) {
    const source = new EventSource(`/api/events?token=${encodeURIComponent(token)}`);
    source.addEventListener("status", (event) => handleStatusEvent(JSON.parse(event.data)));
    source.addEventListener("log", (event) => handleLogEvent(JSON.parse(event.data)));
  }

  ui.form.addEventListener("submit", submit);
  ui.authKind.addEventListener("change", syncAuthFields);
  ui.save.addEventListener("click", saveProfile);
  ui.remove.addEventListener("click", deleteProfile);
  ui.disconnect.addEventListener("click", disconnect);
  ui.open.addEventListener("click", openTunnel);
  ui.copy.addEventListener("click", copyUrl);
  ui.quit.addEventListener("click", quitClient);
  el("browse-key").addEventListener("click", browseKey);
  ui.clearLog.addEventListener("click", () => ui.log.replaceChildren());
  ui.profileSelect.addEventListener("change", () => {
    const profile = selectedProfile();
    ui.remove.disabled = !profile;
    fillForm(profile || {});
  });

  applyStatus(status);
}

init().catch((err) => showError(err.message));



