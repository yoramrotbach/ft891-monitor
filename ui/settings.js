// 891 Monitor settings window. The main window owns the settings: this
// window asks for them on load and sends every change back as an event.

(function () {
  const $ = (id) => document.getElementById(id);
  const T = window.__TAURI__;
  let cfg = null;
  let connectionChanged = false;

  // Settings that change how the app talks to the radio. Changing any of
  // these restarts the connection when the window is closed.
  const CONNECTION_KEYS = new Set(["kind", "port", "baud", "stopBits", "rtsCts", "host", "tcpPort", "allowControl"]);

  function send(key) {
    if (CONNECTION_KEYS.has(key)) {
      connectionChanged = true;
      $("apply-hint").textContent = "Connection changes will apply when you click Done.";
    }
    T.event.emit("settings-changed", { cfg, connection: CONNECTION_KEYS.has(key) });
  }

  async function refreshPorts() {
    const sel = $("port");
    let ports = [];
    try { ports = await T.core.invoke("list_ports"); } catch (_) {}
    sel.innerHTML = "";
    if (!ports.length) {
      const o = document.createElement("option");
      o.value = ""; o.textContent = "No serial ports found";
      sel.appendChild(o);
    }
    for (const p of ports) {
      const o = document.createElement("option");
      o.value = p.name;
      o.textContent = p.description ? `${p.name} — ${p.description}` : p.name;
      sel.appendChild(o);
    }
    if (cfg.port && !ports.some((p) => p.name === cfg.port)) {
      const o = document.createElement("option");
      o.value = cfg.port; o.textContent = `${cfg.port} (not present)`;
      sel.appendChild(o);
    }
    sel.value = cfg.port || (ports[0]?.name ?? "");
    if (sel.value !== cfg.port) { cfg.port = sel.value; send("port"); }
  }

  function syncForm() {
    document.querySelectorAll('input[name="kind"]').forEach((r) => (r.checked = r.value === cfg.kind));
    $("serial-opts").classList.toggle("hidden", cfg.kind !== "serial");
    $("rigctld-opts").classList.toggle("hidden", cfg.kind !== "rigctld");
    $("baud").value = cfg.baud;
    $("host").value = cfg.host;
    $("tcp-port").value = cfg.tcpPort;
    $("stop-bits").value = cfg.stopBits;
    $("rts-cts").checked = cfg.rtsCts;
    $("allow-control").checked = cfg.allowControl;
    $("keys-row").value = cfg.keysRow;
    $("theme").value = cfg.theme;
    $("on-top").checked = cfg.onTop;
  }

  function bind(id, key, ev = "change", conv = (v) => v) {
    $(id).addEventListener(ev, (e) => {
      const el = e.target;
      cfg[key] = conv(el.type === "checkbox" ? el.checked : el.value);
      send(key);
    });
  }

  function wire() {
    document.querySelectorAll('input[name="kind"]').forEach((r) =>
      r.addEventListener("change", () => { cfg.kind = r.value; syncForm(); send("kind"); }));
    bind("port", "port");
    bind("baud", "baud");
    bind("host", "host", "input");
    bind("tcp-port", "tcpPort", "input", Number);
    bind("stop-bits", "stopBits");
    bind("rts-cts", "rtsCts");
    bind("allow-control", "allowControl");
    bind("keys-row", "keysRow"); // applied live in the display window
    bind("theme", "theme");     // applied live in the display window
    bind("on-top", "onTop");    // applied live in the display window
    $("refresh-ports").addEventListener("click", refreshPorts);
    const done = () => T.window.getCurrentWindow().close();
    $("done").addEventListener("click", done);
    document.addEventListener("keydown", (e) => { if (e.key === "Escape") done(); });
  }

  // Get the current settings from the display window, then show them.
  T.event.listen("settings-config", (e) => {
    if (cfg) return; // only the first reply
    cfg = e.payload;
    syncForm();
    refreshPorts();
    wire();
    $("apply-hint").textContent = connectionChanged ? "" : "Display options apply immediately.";
  }).then(() => T.event.emit("settings-request"));
})();
