// 891 Monitor UI. Talks to the Rust back end through Tauri; when opened in a
// plain browser (UI development), it falls back to a small built-in mock.

(function () {
  const $ = (id) => document.getElementById(id);
  const TAURI = window.__TAURI__;
  const invoke = TAURI ? TAURI.core.invoke : mockInvoke;
  const listen = TAURI ? TAURI.event.listen : mockListen;

  // ------------------------------------------------------------ settings
  const FUNCTION_ROWS = {
    "sft-scp-dnr": ["SFT", "SCP", "DNR"],
    "sft-scp-nb": ["SFT", "SCP", "NB"],
    "att-ipo-nar": ["ATT", "IPO", "NAR"],
    "nb-agc-moni": ["NB", "AGC", "MONI"],
    "shift-width-notch": ["SHIFT", "WIDTH", "NOTCH"],
    "mch-grp-mlist": ["MCH", "GRP", "MLIST"],
  };
  const DEFAULTS = {
    kind: "serial", port: "", baud: "38400", stopBits: "two", rtsCts: true,
    host: "127.0.0.1", tcpPort: 4532, allowControl: true, viewOnly: false,
    keysRow: "sft-scp-dnr", theme: "lcd", onTop: false, autoConnect: false,
  };
  let cfg = { ...DEFAULTS };
  try { cfg = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("ft891mon") || "{}") }; } catch (_) {}
  const save = () => { try { localStorage.setItem("ft891mon", JSON.stringify(cfg)); } catch (_) {} };
  const applyTheme = () => {
    document.body.classList.remove("theme-lcd", "theme-dim", "theme-night");
    document.body.classList.add("theme-" + cfg.theme);
    document.body.classList.toggle("view-only", !!cfg.viewOnly);
    $("view-only").textContent = cfg.viewOnly ? "🔒" : "🔓";
    $("view-only").title = cfg.viewOnly ? "View-only is on: click to allow control" : "View-only: block all control from this window";
  };

  // ------------------------------------------------------------ control
  let last = { link: "stopped" };
  let toastTimer = null;
  function toast(msg) {
    const t = $("toast");
    t.textContent = msg;
    t.classList.remove("hidden");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => t.classList.add("hidden"), 3000);
  }
  function controlBlocked() {
    if (cfg.viewOnly) return "View-only is on (🔒)";
    if (!cfg.allowControl) return "Controls are turned off in Settings";
    if (last.link !== "up") return "Not connected";
    if (last.tx) return "Refused: the radio is transmitting";
    return null;
  }
  async function control(action) {
    const why = controlBlocked();
    if (why) { toast(why); return false; }
    try { await invoke("rig_control", { action }); return true; }
    catch (e) { toast(String(e)); return false; }
  }

  // Wheel tuning: accumulate steps locally, show them at once, send the
  // target at most every 80 ms. The radio's answer then takes over again.
  let tuneTarget = null, tuneSent = 0, tuneTimer = null, tuneHoldUntil = 0;
  function tune(stepHz, dir) {
    if (controlBlocked()) { toast(controlBlocked()); return; }
    if (last.lock) { toast("Refused: the radio's dial LOCK is on"); return; }
    const base = tuneTarget ?? last.freq_hz;
    if (base == null) return;
    let next = base + dir * stepHz;
    if (stepHz >= 100) next = Math.round(next / stepHz) * stepHz; // snap like a real dial step
    next = Math.min(56000000, Math.max(30000, next));
    tuneTarget = next;
    tuneHoldUntil = performance.now() + 700;
    drawLcd();
    if (!tuneTimer) {
      const flush = () => {
        tuneTimer = null;
        if (tuneTarget != null && tuneTarget !== tuneSent) {
          tuneSent = tuneTarget;
          control({ op: "set_freq", hz: tuneTarget });
          tuneTimer = setTimeout(flush, 80);
        }
      };
      tuneTimer = setTimeout(flush, 20);
    }
  }

  // ------------------------------------------------------------ LCD
  const lcd = buildLcd($("lcd"), {
    onWheel: (step, dir) => tune(step, dir),
    onFreqClick: () => openFreqEntry(),
    onMode: (e) => openModeMenu(e),
    onToggle: (id) => {
      const map = {
        spl: ["split", "split"], lock: ["lock", "lock"], att: ["attenuator", "att"], nar: ["narrow", "nar"],
        nb: ["noise_blanker", "nb"], fst: ["fast", "fst"],
      };
      if (id === "ipo") return control({ op: "preamp", on: !!last.ipo }); // IPO on → turn the preamp on
      const [op, key] = map[id] || [];
      if (op) control({ op, on: !last[key] });
    },
    onKey: (i) => {
      const label = (FUNCTION_ROWS[cfg.keysRow] || [])[i];
      const toggles = { DNR: ["noise_reduction", "dnr"], NB: ["noise_blanker", "nb"], ATT: ["attenuator", "att"], NAR: ["narrow", "nar"] };
      if (label === "IPO") return control({ op: "preamp", on: !!last.ipo });
      if (toggles[label]) return control({ op: toggles[label][0], on: !last[toggles[label][1]] });
      if (i === 3) return toast("Clarifier settings are changed on the radio");
      if (label) toast(`${label} is changed on the radio`);
    },
  });

  // S-meter peak hold: hold 1.2 s, then decay
  let peak = 0, peakAt = 0;
  function sPeak(v) {
    const now = performance.now();
    if (v >= peak) { peak = v; peakAt = now; }
    else if (now - peakAt > 1200) { peak = Math.max(v, peak - 8); peakAt = now - 1150; }
    return peak;
  }

  function drawLcd() {
    const view = {
      freqOverride: tuneTarget != null && performance.now() < tuneHoldUntil ? tuneTarget : null,
      keysLabels: [...(FUNCTION_ROWS[cfg.keysRow] || ["", "", ""]), "CLA"],
      meterPeak: last.tx ? null : sPeak(last.s_raw ?? 0),
    };
    if (view.freqOverride == null) tuneTarget = null;
    renderLcd(lcd, last, view);
  }

  // ------------------------------------------------------------ frequency entry
  function openFreqEntry() {
    if (controlBlocked()) { toast(controlBlocked()); return; }
    const f = $("freq-entry"), inp = $("freq-input");
    f.classList.remove("hidden");
    inp.value = last.freq_hz ? (last.freq_hz / 1e6).toFixed(6).replace(/0+$/, "").replace(/\.$/, "") : "";
    inp.focus(); inp.select();
  }
  function closeFreqEntry() { $("freq-entry").classList.add("hidden"); }
  $("freq-entry").addEventListener("submit", (e) => {
    e.preventDefault();
    const raw = $("freq-input").value.trim().replace(",", ".");
    let hz = Number(raw);
    if (!isFinite(hz) || hz <= 0) { toast("Type a frequency in MHz, e.g. 14.074 (or in kHz, e.g. 7074)"); return; }
    hz = hz < 100 ? hz * 1e6 : hz < 100000 ? hz * 1e3 : hz; // MHz, kHz or Hz
    hz = Math.round(hz);
    if (hz < 30000 || hz > 56000000) { toast("The FT-891 covers 0.03–56 MHz"); return; }
    if (last.lock) { toast("Refused: the radio's dial LOCK is on"); return; }
    control({ op: "set_freq", hz });
    closeFreqEntry();
  });
  $("freq-input").addEventListener("keydown", (e) => { if (e.key === "Escape") closeFreqEntry(); });

  // ------------------------------------------------------------ mode menu
  let modes = ["LSB", "USB", "CWU", "CWL", "AM", "AM-N", "FM", "FM-N", "R-L", "R-U", "D-L", "D-U"];
  invoke("list_modes").then((m) => { if (Array.isArray(m) && m.length) modes = m; }).catch(() => {});
  function openModeMenu(e) {
    if (controlBlocked()) { toast(controlBlocked()); return; }
    const menu = $("mode-menu");
    menu.innerHTML = "";
    for (const m of modes) {
      const b = document.createElement("button");
      b.textContent = m;
      if (m === last.mode) b.className = "cur";
      b.addEventListener("click", () => { closeModeMenu(); control({ op: "set_mode", mode: m }); });
      menu.appendChild(b);
    }
    const wrap = $("lcd-wrap").getBoundingClientRect();
    const x = e && e.clientX ? e.clientX - wrap.left : 40;
    const y = e && e.clientY ? e.clientY - wrap.top : 40;
    menu.style.left = Math.max(8, Math.min(x, wrap.width - 260)) + "px";
    menu.style.top = Math.max(8, Math.min(y, wrap.height - 120)) + "px";
    menu.classList.remove("hidden");
  }
  function closeModeMenu() { $("mode-menu").classList.add("hidden"); }
  document.addEventListener("mousedown", (e) => {
    if (!$("mode-menu").contains(e.target) && !e.target.closest?.(".mode") && !e.target.closest?.('[data-act="mode"]')) closeModeMenu();
  });

  // ------------------------------------------------------------ keys
  $("keys").addEventListener("click", (e) => {
    const k = e.target.closest("[data-act]");
    if (!k) return;
    switch (k.dataset.act) {
      case "mode": {
        const r = k.getBoundingClientRect();
        return $("mode-menu").classList.contains("hidden") ? openModeMenu({ clientX: r.left, clientY: r.top - 110 }) : closeModeMenu();
      }
      case "band-up": return control({ op: "band_up" });
      case "band-down": return control({ op: "band_down" });
      case "swap": return control({ op: "swap_ab" });
      case "copy": return control({ op: "copy_a_to_b" });
      case "split": return control({ op: "split", on: !last.split });
      case "dnr": return control({ op: "noise_reduction", on: !last.dnr });
      case "view-only": cfg.viewOnly = !cfg.viewOnly; save(); applyTheme(); return;
    }
  });
  let pwrDragging = false;
  $("pwr").addEventListener("input", () => { pwrDragging = true; $("pwr-val").textContent = $("pwr").value + " W"; });
  $("pwr").addEventListener("change", async () => {
    pwrDragging = false;
    await control({ op: "power", watts: Number($("pwr").value) });
  });

  // Keyboard: ↑/↓ tune 10 Hz (Shift 100 Hz, Ctrl 1 kHz), PgUp/PgDn change band.
  document.addEventListener("keydown", (e) => {
    if (e.target.tagName === "INPUT") return;
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      const step = e.ctrlKey || e.metaKey ? 1000 : e.shiftKey ? 100 : 10;
      tune(step, e.key === "ArrowUp" ? 1 : -1);
    } else if (e.key === "PageUp") { e.preventDefault(); control({ op: "band_up" }); }
    else if (e.key === "PageDown") { e.preventDefault(); control({ op: "band_down" }); }
    else if (e.key === "Escape") { closeFreqEntry(); closeModeMenu(); }
  });

  // ------------------------------------------------------------ render
  let running = false;
  function render(s) {
    last = s;
    $("dot").className = "dot " + s.link;
    $("link-text").textContent = s.link === "lost" && s.retry_in ? `No link · retrying in ${s.retry_in} s` : { up: "Connected", connecting: "Connecting…", lost: "No link", stopped: "Stopped" }[s.link];
    $("source").textContent = s.source || "Not connected";
    $("rate").textContent = s.link === "up" ? `${Math.round(s.polls_per_sec)} polls/s` : "";
    running = s.link !== "stopped";
    $("connect-btn").textContent = running ? "Disconnect" : "Connect";

    const down = s.link !== "up";
    $("nolink").classList.toggle("hidden", !down);
    $("nolink-title").textContent = s.link === "connecting" && s.error ? "NO LINK" : ({ connecting: "CONNECTING", lost: "NO LINK", stopped: "STANDBY" }[s.link] || "");
    if (s.link === "stopped") {
      $("nolink-msg").textContent = "Press Connect, or open Settings (⚙) to choose the port.";
      $("nolink-status").textContent = "";
    } else {
      $("nolink-msg").textContent = s.error || "";
      $("nolink-status").textContent = s.message || "";
    }

    // LED: red on transmit, green when the receiver is busy (squelch open)
    const led = $("led");
    led.className = "led" + (!down && s.tx ? " tx" : !down && s.busy ? " busy" : "");

    if (s.control_note) toast(s.control_note);

    const ex = [];
    if (!down) {
      if (s.band) ex.push(s.band);
      if (s.agc) ex.push("AGC " + s.agc);
      if (s.tx && s.po_watts != null) ex.push(`PO ${Math.round(s.po_watts)} W`);
      if (s.tx && s.swr != null) ex.push(`SWR ${s.swr.toFixed(1)}`);
      if (!s.tx && s.s_label) ex.push(s.s_label);
    }
    $("extras").textContent = ex.join("  ");
    if (s.power_w != null && !pwrDragging) {
      $("pwr").value = s.power_w;
      $("pwr-val").textContent = s.power_w + " W";
    }
    $("keys").querySelector('[data-act="split"]').classList.toggle("active", !!s.split);
    $("keys").querySelector('[data-act="dnr"]').classList.toggle("active", !!s.dnr);
    const noCtl = !cfg.allowControl || !s.can_control;
    $("keys").querySelectorAll(".key:not(.lock-btn)").forEach((b) => (b.disabled = noCtl && s.link === "up"));
    drawLcd();
  }

  // ------------------------------------------------------------ connect
  function monitorConfig() {
    const source = cfg.kind === "serial"
      ? { kind: "serial", port: cfg.port, baud: Number(cfg.baud), stop_bits: cfg.stopBits, rts_cts: !!cfg.rtsCts }
      : cfg.kind === "rigctld"
        ? { kind: "rigctld", host: cfg.host, port: Number(cfg.tcpPort) }
        : { kind: "demo" };
    return { source, allow_control: !!cfg.allowControl };
  }
  async function connect() {
    if (cfg.kind === "serial" && !cfg.port) { openSettings(); return; }
    cfg.autoConnect = true; save();
    try { await invoke("start_monitor", { config: monitorConfig() }); }
    catch (e) { render({ link: "lost", error: String(e), message: "", source: "" }); }
  }
  async function disconnect() {
    cfg.autoConnect = false; save();
    await invoke("stop_monitor");
  }
  $("connect-btn").addEventListener("click", () => (running ? disconnect() : connect()));

  // ------------------------------------------------------------ settings window
  const openSettings = () => invoke("open_settings").catch(() => {});
  $("settings-btn").addEventListener("click", openSettings);
  let pendingReconnect = false;
  listen("settings-request", () => { if (TAURI) TAURI.event.emit("settings-config", cfg); });
  listen("settings-changed", (e) => {
    const { cfg: next, connection } = e.payload;
    const onTopChanged = next.onTop !== cfg.onTop;
    cfg = { ...cfg, ...next, autoConnect: cfg.autoConnect, viewOnly: cfg.viewOnly };
    save();
    applyTheme();
    drawLcd();
    if (onTopChanged && TAURI) TAURI.window.getCurrentWindow().setAlwaysOnTop(!!cfg.onTop).catch(() => {});
    if (connection) pendingReconnect = true;
  });
  listen("settings-closed", async () => {
    if (pendingReconnect && running) await connect();
    pendingReconnect = false;
  });

  // ------------------------------------------------------------ start
  applyTheme();
  render({ link: "stopped", source: "", message: "" });
  listen("rig-state", (e) => render(e.payload));
  if (TAURI && cfg.onTop) TAURI.window.getCurrentWindow().setAlwaysOnTop(true).catch(() => {});
  if (cfg.autoConnect || /[?&]demo/.test(location.search)) {
    if (/[?&]demo/.test(location.search)) cfg.kind = "demo";
    connect();
  }

  // ------------------------------------------------------------ browser mock
  function mockListen(name, cb) { if (name === "rig-state") mockListen.cb = cb; return Promise.resolve(() => {}); }
  async function mockInvoke(cmd, args) {
    if (cmd === "open_settings") return;
    if (cmd === "list_modes") return modes;
    if (cmd === "list_ports") return [{ name: "/dev/ttyUSB0", description: "Silicon Labs CP2105 Dual USB to UART Bridge Controller" }];
    if (cmd === "rig_control") { Object.assign(mockInvoke.st, mockApply(args.action)); return; }
    if (cmd === "stop_monitor") { clearInterval(mockInvoke.t); mockListen.cb?.({ payload: { link: "stopped" } }); return; }
    if (cmd === "start_monitor") {
      clearInterval(mockInvoke.t);
      const t0 = performance.now();
      mockInvoke.st = { freq_hz: 14074000, freq_b_hz: 14076500, mode: "D-U", mode_b: "D-U", split: false, nb: false, dnr: true, att: false, nar: false, ipo: false, fst: false, lock: false, power_w: 50 };
      mockInvoke.t = setInterval(() => {
        const t = ((performance.now() - t0) / 1000) % 20;
        const tx = /[?&]tx/.test(location.search) || t > 13;
        const sr = Math.round(80 + 50 * Math.sin(t * 0.8) + 25 * Math.abs(Math.sin(t * 7)));
        mockListen.cb?.({ payload: {
          ...mockInvoke.st, link: "up", source: "Browser mock", polls_per_sec: 31, errors: 0, message: "", can_control: true,
          band: "20m", vfo_mem: "VFO", tx, busy: !tx && sr > 110, s_raw: tx ? 0 : sr, s_label: "S7",
          meter: "PO", meter_raw: tx ? 120 + Math.round(6 * Math.sin(t * 9)) : 0, po_watts: tx ? 48 : null, swr: tx ? 1.2 : null,
          tnr: true, vox: false, prc: false, mon: false, agc: "AUTO-F", clar_on: false, clar_hz: 0,
        } });
      }, 50);
    }
  }
  function mockApply(a) {
    const s = mockInvoke.st;
    switch (a.op) {
      case "set_freq": return { freq_hz: a.hz };
      case "set_mode": return { mode: a.mode };
      case "swap_ab": return { freq_hz: s.freq_b_hz, freq_b_hz: s.freq_hz, mode: s.mode_b, mode_b: s.mode };
      case "copy_a_to_b": return { freq_b_hz: s.freq_hz, mode_b: s.mode };
      case "split": return { split: a.on };
      case "lock": return { lock: a.on };
      case "noise_blanker": return { nb: a.on };
      case "noise_reduction": return { dnr: a.on };
      case "narrow": return { nar: a.on };
      case "attenuator": return { att: a.on };
      case "preamp": return { ipo: !a.on };
      case "fast": return { fst: a.on };
      case "power": return { power_w: a.watts };
      default: return {};
    }
  }
})();
