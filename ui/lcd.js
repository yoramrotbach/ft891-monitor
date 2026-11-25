// FT-891-style LCD drawn as one SVG: indicator columns on both sides, the
// big VFO-A frequency, the meter with its scale, the VFO-B row and the
// function-key labels. Unlit segments stay faintly visible, like a real LCD.
//
// The layout follows the radio's display; the drawing itself is original.

(function () {
  const NS = "http://www.w3.org/2000/svg";
  const VW = 760, VH = 252;

  function el(tag, attrs, parent) {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
    if (parent) parent.appendChild(e);
    return e;
  }

  // ------------------------------------------------------------ 7 segments
  // Digit cell is drawn in a 50 x 100 box, then scaled.
  const T = 10, G = 1.8, W = 50, H = 100;
  function hseg(x1, x2, y) {
    const h = T / 2;
    return [[x1, y], [x1 + h, y - h], [x2 - h, y - h], [x2, y], [x2 - h, y + h], [x1 + h, y + h]];
  }
  function vseg(x, y1, y2) {
    const h = T / 2;
    return [[x, y1], [x + h, y1 + h], [x + h, y2 - h], [x, y2], [x - h, y2 - h], [x - h, y1 + h]];
  }
  const x0 = T / 2, x1 = W - T / 2, ym = H / 2;
  const SEG = {
    a: hseg(x0 + G, x1 - G, T / 2),
    g: hseg(x0 + G, x1 - G, ym),
    d: hseg(x0 + G, x1 - G, H - T / 2),
    f: vseg(x0, T / 2 + G, ym - G),
    b: vseg(x1, T / 2 + G, ym - G),
    e: vseg(x0, ym + G, H - T / 2 - G),
    c: vseg(x1, ym + G, H - T / 2 - G),
  };
  const DIGITS = {
    "0": "abcdef", "1": "bc", "2": "abged", "3": "abgcd", "4": "fgbc",
    "5": "afgcd", "6": "afgedc", "7": "abc", "8": "abcdefg", "9": "abcdfg",
    "-": "g", " ": "",
  };

  // A run of digits. layout: string of "d" (digit) and "." (dot).
  // Returns { set(chars[]), digitRects: [{x,w,index}] } in SVG units.
  function segRun(parent, x, y, height, layout, opts = {}) {
    const s = height / H;
    const cell = (opts.cell ?? 62) * s, dot = (opts.dot ?? 20) * s;
    const g = el("g", { class: "segrun" }, parent);
    const slots = [], rects = [];
    let cx = x;
    for (const kind of layout) {
      if (kind === ".") {
        el("rect", { x: cx + 2 * s, y: y + height - 11 * s, width: 10 * s, height: 11 * s, class: "on" }, g);
        cx += dot;
        continue;
      }
      const dg = el("g", { transform: `translate(${cx + 5 * s},${y}) scale(${s}) skewX(-6)` }, g);
      const parts = {};
      for (const [name, pts] of Object.entries(SEG)) {
        parts[name] = el("polygon", { points: pts.map(([a, b]) => `${a},${b}`).join(" "), class: "off" }, dg);
      }
      rects.push({ x: cx, w: cell, index: slots.length });
      slots.push(parts);
      cx += cell;
    }
    return {
      end: cx,
      rects,
      set(chars) {
        slots.forEach((parts, i) => {
          const lit = DIGITS[chars[i]] ?? "";
          for (const [n, p] of Object.entries(parts)) p.setAttribute("class", lit.includes(n) ? "on" : "off");
        });
      },
    };
  }

  // ------------------------------------------------------------ text boxes
  // Indicator: outlined box with a label. state: "on" | "off" | "rev" (reversed).
  function box(parent, x, y, w, h, text, size, id) {
    const g = el("g", { class: "ind off", id: id ? "ind-" + id : undefined }, parent);
    el("rect", { x, y, width: w, height: h, rx: 2, class: "frame" }, g);
    const t = el("text", { x: x + w / 2, y: y + h / 2 + size * 0.36, "font-size": size, "text-anchor": "middle", class: "label" }, g);
    t.textContent = text;
    return { g, t, setState(s) { g.setAttribute("class", "ind " + s + (g.dataset.click ? " clickable" : "")); }, setText(s) { t.textContent = s; } };
  }

  function text(parent, x, y, size, str, cls = "on", anchor = "start") {
    const t = el("text", { x, y, "font-size": size, "text-anchor": anchor, class: "txt " + cls }, parent);
    t.textContent = str;
    return t;
  }

  // ------------------------------------------------------------ meter scales
  // Each scale: label marks at fractions of the bar width.
  const SCALES = {
    S: { marks: [["1", 0.05], ["3", 0.15], ["5", 0.25], ["7", 0.35], ["9", 0.45], ["20", 0.6], ["40", 0.75], ["60", 0.9]], unit: "dB" },
    PO: { marks: [["0", 0.0], ["10", 0.16], ["25", 0.34], ["50", 0.55], ["75", 0.73], ["100", 0.9]], unit: "W" },
    SWR: { marks: [["1", 0.0], ["1.5", 0.2], ["2", 0.35], ["3", 0.5], ["5", 0.72], ["∞", 0.97]], unit: "" },
    ALC: { marks: [["0", 0.0], ["", 0.25], ["", 0.5], ["MAX", 0.92]], unit: "" },
    COMP: { marks: [["0", 0.0], ["5", 0.2], ["10", 0.4], ["15", 0.6], ["20", 0.8]], unit: "dB" },
    ID: { marks: [["0", 0.0], ["5", 0.22], ["10", 0.45], ["15", 0.68], ["20", 0.9]], unit: "A" },
  };
  const NSEG = 40;

  // S raw (0..255) to bar fraction, matching the S scale above: S9 (raw 130)
  // sits at 0.45, +60 dB (raw 255) at the end. Uses Hamlib's calibration.
  const S_CAL = [[0, -54], [12, -48], [27, -42], [40, -36], [55, -30], [65, -24], [80, -18], [95, -12], [112, -6], [130, 0], [150, 10], [172, 20], [190, 30], [220, 40], [240, 50], [255, 60]];
  function sDb(raw) {
    if (raw <= 0) return -54;
    for (let i = 1; i < S_CAL.length; i++) {
      const [x0, y0] = S_CAL[i - 1], [x1, y1] = S_CAL[i];
      if (raw <= x1) return y0 + (y1 - y0) * (raw - x0) / (x1 - x0);
    }
    return 60;
  }
  function sFraction(raw) {
    const db = sDb(raw);
    // S1 (-48 dB) at 0.05 … S9 (0 dB) at 0.45, then +60 dB at 0.955
    if (db <= 0) return Math.max(0, 0.05 + (db + 48) / 48 * 0.40);
    return 0.45 + db / 60 * 0.45;
  }
  // Transmit meters: raw 0..255 maps across the bar (PO uses Hamlib's watts cal).
  const PO_CAL = [[0, 0], [10, 0.8], [50, 8], [100, 26], [150, 54], [200, 92], [250, 140]];
  function poWatts(raw) {
    for (let i = 1; i < PO_CAL.length; i++) {
      const [x0, y0] = PO_CAL[i - 1], [x1, y1] = PO_CAL[i];
      if (raw <= x1) return y0 + (y1 - y0) * (raw - x0) / (x1 - x0);
    }
    return 140;
  }
  function wattsFraction(w) {
    const pts = [[0, 0], [10, 0.16], [25, 0.34], [50, 0.55], [75, 0.73], [100, 0.9], [140, 1]];
    for (let i = 1; i < pts.length; i++) {
      const [x0, y0] = pts[i - 1], [x1, y1] = pts[i];
      if (w <= x1) return y0 + (y1 - y0) * (w - x0) / (x1 - x0);
    }
    return 1;
  }
  function txFraction(meter, raw) {
    if (meter === "PO") return wattsFraction(poWatts(raw));
    return Math.min(1, raw / 255);
  }

  // ------------------------------------------------------------ build
  window.buildLcd = function (svg, handlers) {
    svg.innerHTML = "";
    svg.setAttribute("viewBox", `0 0 ${VW} ${VH}`);
    const root = el("g", {}, svg);
    const api = { handlers };

    // Side indicator columns
    const L = [["tnr", "TNR"], ["vox", "VOX"], ["prc", "PRC"], ["mon", "MON"], ["spl", "SPL"]];
    const R = [["ipo", "IPO"], ["att", "ATT"], ["nar", "NAR"], ["nb", "NB"], ["fst", "FST"]];
    api.ind = {};
    L.forEach(([id, t], i) => (api.ind[id] = box(root, 8, 8 + i * 40, 74, 32, t, 25, id)));
    R.forEach(([id, t], i) => (api.ind[id] = box(root, 678, 8 + i * 40, 74, 32, t, 25, id)));
    api.ind.lock = box(root, 8, 210, 92, 34, "LOCK", 26, "lock");

    // VFO box ("A", or "M" in memory mode) and mode
    api.vfo = box(root, 92, 8, 62, 40, "A", 34, "vfo");
    api.vfo.setState("rev");
    api.mem = text(root, 160, 22, 15, "", "on");
    api.mode = text(root, 123, 82, 30, "USB", "on mode clickable", "middle");
    api.mode.addEventListener("click", (e) => handlers.onMode?.(e));

    // Big frequency: " MM.kkk.hh" + small 1 Hz digit
    const fg = el("g", { class: "freq clickable" }, root);
    api.big = segRun(fg, 176, 10, 80, "dd.ddd.dd", { cell: 62, dot: 22 });
    api.small = segRun(fg, api.big.end + 2, 46, 44, "d", { cell: 62 });
    api.freqBox = { x0: 176, x1: api.small.end, y0: 6, y1: 94 };
    // Digit place values for wheel tuning (Hz): 10M,1M,100k,10k,1k,100,10 + 1
    api.places = [10e6, 1e6, 1e5, 1e4, 1e3, 100, 10];
    const hit = el("rect", { x: 170, y: 4, width: api.small.end - 168, height: 92, class: "hit" }, fg);
    hit.addEventListener("wheel", (e) => {
      e.preventDefault();
      const pt = toSvg(svg, e);
      handlers.onWheel?.(placeAt(api, pt.x), e.deltaY < 0 ? 1 : -1);
    }, { passive: false });
    hit.addEventListener("click", (e) => handlers.onFreqClick?.(e));

    // Meter: selected-meter box, scale and bar
    api.meterBox = box(root, 92, 98, 62, 32, "PO", 22, "meter");
    api.meterBox.setState("rev");
    api.scaleG = el("g", { class: "scale" }, root);
    api.bar = [];
    const bx0 = 168, bx1 = 664, by = 124, bh = 15;
    api.barGeom = { bx0, bx1 };
    const step = (bx1 - bx0) / NSEG;
    for (let i = 0; i < NSEG; i++) {
      api.bar.push(el("rect", { x: bx0 + i * step + 0.8, y: by, width: step - 2.2, height: bh, class: "off" }, root));
    }
    el("rect", { x: bx0, y: by + bh + 3, width: bx1 - bx0, height: 1.6, class: "on" }, root);

    // VFO-B row
    api.bLetter = text(root, 112, 192, 38, "B", "on", "middle");
    api.bMode = text(root, 140, 190, 20, "USB", "on");
    const bg = el("g", { class: "freqb" }, root);
    api.bFreq = segRun(bg, 220, 156, 38, "dd.ddd.ddd", { cell: 62, dot: 22 });
    api.clar = text(root, 664, 190, 18, "", "on", "end");

    // Function-key labels
    api.keys = [];
    const kx = 126, kw = 126, kgap = 12;
    for (let i = 0; i < 4; i++) {
      const b = box(root, kx + i * (kw + kgap), 210, kw, 34, "", 24, "key" + i);
      b.setState("on");
      api.keys.push(b);
    }

    // Clickable indicators
    for (const id of ["spl", "lock", "ipo", "att", "nar", "nb", "fst"]) {
      const g = api.ind[id].g;
      g.dataset.click = "1";
      g.addEventListener("click", () => handlers.onToggle?.(id));
    }
    api.keys.forEach((k, i) => {
      k.g.dataset.click = "1";
      k.g.addEventListener("click", () => handlers.onKey?.(i));
    });

    api.drawScale = (name) => drawScale(api, name);
    api.drawScale("S");
    return api;
  };

  function toSvg(svg, e) {
    const p = svg.createSVGPoint();
    p.x = e.clientX; p.y = e.clientY;
    return p.matrixTransform(svg.getScreenCTM().inverse());
  }

  function placeAt(api, x) {
    const r = api.big.rects.find((r) => x >= r.x && x < r.x + r.w);
    if (r) return api.places[r.index];
    if (x >= api.big.end) return 10; // small digit: still tune in 10 Hz steps
    return 1e3;
  }

  function drawScale(api, name) {
    if (api.scaleName === name) return;
    api.scaleName = name;
    const g = api.scaleG;
    g.innerHTML = "";
    const sc = SCALES[name] || SCALES.S;
    const { bx0, bx1 } = api.barGeom;
    const y = 117;
    // dashes between the numbers, like the radio's "1_3_5_7_9_20_40_60"
    el("rect", { x: bx0, y: y - 1, width: bx1 - bx0 - 30, height: 1.4, class: "on dash" }, g);
    for (const [lbl, f] of sc.marks) {
      if (!lbl) continue;
      const x = bx0 + f * (bx1 - bx0);
      const t = text(g, x, y, 17, lbl, "on scaletxt", "middle");
      t.setAttribute("paint-order", "stroke");
    }
    if (sc.unit) {
      const [lbl, f] = sc.marks[sc.marks.length - 1];
      const x = bx0 + f * (bx1 - bx0) + lbl.length * 5 + 3;
      text(g, x, y, 15, sc.unit, "on scaletxt", "start");
    }
  }

  // ------------------------------------------------------------ render
  function freqChars(hz) {
    if (hz == null) return { big: "--------".split(""), small: ["-"] };
    hz = Math.round(hz);
    const s = String(hz).padStart(8, " "); // up to 56 MHz = 8 digits
    return { big: s.slice(0, 7).split(""), small: [s[7]] };
  }

  window.renderLcd = function (api, st, view) {
    // view: { freqOverride, keysLabels, meterPeak }
    const hz = view.freqOverride ?? st.freq_hz;
    const f = freqChars(hz);
    api.big.set(f.big);
    api.small.set(f.small);

    const mem = st.vfo_mem && st.vfo_mem !== "VFO";
    api.vfo.setText(mem ? "M" : "A");
    api.mem.textContent = mem && st.mem_channel ? st.mem_channel : "";
    api.mode.textContent = st.mode ?? "";

    // VFO-B row (or the clarifier offset while CLAR is on)
    if (st.freq_b_hz != null) {
      const b = String(Math.round(st.freq_b_hz)).padStart(8, " ").split("");
      api.bFreq.set(b);
    } else {
      api.bFreq.set("--------".split(""));
    }
    api.bMode.textContent = st.mode_b ?? "";
    api.clar.textContent = st.clar_on ? `CLAR ${st.clar_hz >= 0 ? "+" : "−"}${Math.abs(st.clar_hz)}` : "";

    // Indicators
    const ind = (id, on) => api.ind[id].setState(on == null ? "off" : on ? "on" : "off");
    ind("tnr", st.tnr); ind("vox", st.vox); ind("prc", st.prc); ind("mon", st.mon);
    ind("spl", st.split); ind("ipo", st.ipo); ind("att", st.att); ind("nar", st.nar);
    ind("nb", st.nb); ind("fst", st.fst);
    api.ind.lock.setState(st.lock ? "rev" : "off");

    // Meter
    api.meterBox.setText(st.meter ?? "PO");
    let frac = 0, peakFrac = null;
    if (st.tx) {
      api.drawScale(st.meter ?? "PO");
      frac = st.meter_raw != null ? txFraction(st.meter ?? "PO", st.meter_raw) : 0;
    } else {
      api.drawScale("S");
      frac = st.s_raw != null ? sFraction(st.s_raw) : 0;
      peakFrac = view.meterPeak != null ? sFraction(view.meterPeak) : null;
    }
    const lit = Math.round(frac * NSEG);
    const peakSeg = peakFrac != null ? Math.round(peakFrac * NSEG) : -1;
    api.bar.forEach((r, i) => r.setAttribute("class", i < lit ? "on" : i === peakSeg - 1 ? "on peak" : "off"));

    // Function-key labels
    (view.keysLabels || []).forEach((t, i) => api.keys[i] && api.keys[i].setText(t));
  };
})();
