// Bubba DJ Pro — interfaz
"use strict";

const CFG = window.BUBBA || { wsPort: 8081, token: "" };
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

const store = {
  get(key, def) {
    try {
      const v = localStorage.getItem("bubba." + key);
      return v === null ? def : JSON.parse(v);
    } catch { return def; }
  },
  set(key, value) {
    try { localStorage.setItem("bubba." + key, JSON.stringify(value)); } catch { /* sin almacenamiento */ }
  },
};

const ICON = {
  play: '<svg viewBox="0 0 24 24"><path d="M7 4.5v15l12.5-7.5z" fill="currentColor"/></svg>',
  pause: '<svg viewBox="0 0 24 24"><rect x="6" y="4.5" width="4.2" height="15" rx="1" fill="currentColor"/><rect x="13.8" y="4.5" width="4.2" height="15" rx="1" fill="currentColor"/></svg>',
  headphones: '<svg viewBox="0 0 24 24"><path d="M4 14v-2a8 8 0 0 1 16 0v2" fill="none" stroke="currentColor" stroke-width="2"/><rect x="3" y="13" width="5" height="7" rx="1.5" fill="currentColor"/><rect x="16" y="13" width="5" height="7" rx="1.5" fill="currentColor"/></svg>',
  folder: "▸", music: "♪", download: "↓", desktop: "▭", drive: "◫", cloud: "☁", flask: "✦",
};

const PAD_COLORS = ["#ff4d5e", "#ff8a2e", "#f5c524", "#2bd46b", "#22c3d9", "#2e7bff", "#9b6bff", "#ff5fb2"];
const ZOOMS = [40, 70, 110, 160, 230, 330, 470];
/** Suggested cue points from the phrase analysis */
const CUE_STYLE = {
  Start: { label: "INICIO", color: "#2bd46b" },
  Drop: { label: "DROP", color: "#ff4d5e" },
  Break: { label: "BREAK", color: "#22c3d9" },
  Outro: { label: "OUTRO", color: "#b07cff" },
};
const DECK_NAME = ["1", "2"];

// ======================================================================
// State
// ======================================================================

const S = {
  ws: null,
  connected: false,
  mixer: null,
  recvAt: 0,
  tracks: [null, null],
  loading: [null, null],
  routing: null,
  midi: null,
  zoomIdx: clamp(store.get("zoom", 3), 0, ZOOMS.length - 1),
  timeMode: store.get("timeMode", ["elapsed", "elapsed"]),
  padMode: store.get("padMode", ["hotcue", "hotcue"]),
  shift: false,
  seekHintShown: false,
};

// ======================================================================
// WebSocket
// ======================================================================

let offlineTimer = null;

function connect() {
  const ws = new WebSocket(`ws://127.0.0.1:${CFG.wsPort}/?t=${CFG.token}`);
  S.ws = ws;
  ws.onopen = () => {
    S.connected = true;
    clearTimeout(offlineTimer);
    $("#offline").classList.add("hidden");
  };
  ws.onmessage = (ev) => {
    let msg;
    try { msg = JSON.parse(ev.data); } catch { return; }
    handleMessage(msg);
  };
  ws.onclose = () => {
    S.connected = false;
    clearTimeout(offlineTimer);
    offlineTimer = setTimeout(() => $("#offline").classList.remove("hidden"), 700);
    setTimeout(connect, 1000);
  };
}

function send(cmd) {
  if (S.ws && S.ws.readyState === WebSocket.OPEN) S.ws.send(JSON.stringify(cmd));
}

function handleMessage(msg) {
  switch (msg.type) {
    case "state":
      S.mixer = msg.state;
      S.recvAt = performance.now();
      updateFromState();
      break;
    case "track": onTrack(msg); break;
    case "grid": onGrid(msg); break;
    case "loading":
      S.loading[msg.deck] = msg.name;
      updateDeckHeader(msg.deck);
      break;
    case "loading_failed":
      S.loading[msg.deck] = null;
      updateDeckHeader(msg.deck);
      break;
    case "toast": toast(msg.message, msg.level); break;
    case "audio_routing": onRouting(msg); break;
    case "midi_status": onMidi(msg); break;
    case "midi_ui":
      if (msg.action === "browse") Lib.moveSel(msg.delta);
      if (msg.action === "load") Lib.loadSelected(msg.deck);
      break;
  }
}

function deckState(i) {
  if (!S.mixer) return null;
  return i === 0 ? S.mixer.deck1 : S.mixer.deck2;
}

/** Smoothly interpolated playhead (telemetry arrives ~60-100 times per second) */
function estPos(i) {
  const d = deckState(i);
  if (!d) return 0;
  const dt = Math.min(0.12, (performance.now() - S.recvAt) / 1000);
  let p = d.position_seconds + dt * d.speed;
  if (d.loop_active && p >= d.loop_end_seconds && d.loop_end_seconds > d.loop_start_seconds) {
    p = d.loop_start_seconds + ((p - d.loop_start_seconds) % (d.loop_end_seconds - d.loop_start_seconds));
  }
  return clamp(p, 0, d.duration_seconds || 0);
}

function onTrack(msg) {
  const bin = atob(msg.waveform || "");
  const frames = new Uint8Array(bin.length);
  for (let k = 0; k < bin.length; k++) frames[k] = bin.charCodeAt(k);
  S.tracks[msg.deck] = {
    path: msg.path,
    title: msg.title,
    artist: msg.artist,
    bpm: msg.bpm,
    camelot: msg.camelot,
    key: msg.key,
    duration: msg.duration,
    fps: msg.fps || 150,
    frames,
    grid: msg.beatgrid,
    cues: msg.cues || [],
    refining: !!msg.refining,
    overview: null,
  };
  S.loading[msg.deck] = null;
  updateDeckHeader(msg.deck);
  updateKeyCompat();
  History.add(msg);
  Lib.refreshLoadedMarks();
}

/** Precise grid computed in the background after the track was loaded */
function onGrid(msg) {
  const t = S.tracks[msg.deck];
  if (!t || t.path !== msg.path) return;
  const before = t.bpm;
  Object.assign(t, { grid: msg.beatgrid, cues: msg.cues || [], bpm: msg.bpm, refining: false });
  updateDeckHeader(msg.deck);
  if (before && Math.abs(before - msg.bpm) >= 0.05) {
    toast(`Deck ${DECK_NAME[msg.deck]}: rejilla corregida ${before.toFixed(2)} → ${msg.bpm.toFixed(2)} BPM`, "ok");
  }
}

// ======================================================================
// Helpers
// ======================================================================

function setText(el, txt) {
  if (el._t !== txt) {
    el.textContent = txt;
    el._t = txt;
  }
}

function fmtTime(sec, tenths = true) {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  const base = `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return tenths ? `${base}.${Math.floor((sec % 1) * 10)}` : base;
}

function fmtPct(p) {
  return `${p >= 0 ? "+" : "−"}${Math.abs(p).toFixed(2)}%`;
}

function parseCamelot(c) {
  const m = /^(\d{1,2})([AB])$/.exec(c || "");
  return m ? { n: +m[1], l: m[2] } : null;
}

function camelotColor(c) {
  const k = parseCamelot(c);
  if (!k) return null;
  const hue = ((k.n - 1) * 30 + 160) % 360;
  return k.l === "A" ? `hsl(${hue} 62% 58%)` : `hsl(${hue} 70% 68%)`;
}

function keyRelation(a, b) {
  const x = parseCamelot(a), y = parseCamelot(b);
  if (!x || !y) return null;
  if (x.n === y.n && x.l === y.l) return { ok: true, text: "Mismo tono" };
  if (x.n === y.n) return { ok: true, text: "Relativo mayor/menor" };
  const diff = ((y.n - x.n) % 12 + 12) % 12;
  if (x.l === y.l && (diff === 1 || diff === 11)) return { ok: true, text: diff === 1 ? "+1 energía" : "−1 energía" };
  if (x.l === y.l && (diff === 2)) return { ok: true, text: "+2 (energy boost)" };
  return { ok: false, text: "Choque armónico" };
}

function toast(message, level = "info", ms = 4200) {
  const box = $("#toasts");
  const t = document.createElement("div");
  t.className = `toast ${level}`;
  t.textContent = message;
  box.appendChild(t);
  while (box.children.length > 4) box.firstChild.remove();
  setTimeout(() => {
    t.classList.add("out");
    setTimeout(() => t.remove(), 300);
  }, ms);
}

function api(path, body) {
  const opts = { headers: { "X-Bubba-Token": CFG.token } };
  if (body !== undefined) {
    opts.method = "POST";
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  return fetch(path, opts).then(async (r) => {
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.error || `HTTP ${r.status}`);
    return data;
  });
}

function loadPath(deck, path) {
  send({ action: "LoadTrackFile", deck, path, force: S.shift });
}

function uploadFile(deck, file) {
  toast(`Importando "${file.name}"…`, "info", 2500);
  fetch(`/api/upload?deck=${deck}&force=${S.shift ? 1 : 0}`, {
    method: "POST",
    headers: { "X-Bubba-Token": CFG.token, "X-File-Name": encodeURIComponent(file.name) },
    body: file,
  }).then((r) => {
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
  }).catch((e) => toast(`No se pudo importar el archivo: ${e.message}`, "error"));
}

// ======================================================================
// Controls: Knob, Fader, VU meter
// ======================================================================

function polar(r, deg) {
  const a = (deg - 90) * Math.PI / 180;
  return [20 + r * Math.cos(a), 20 + r * Math.sin(a)];
}

function arcPath(r, a0, a1) {
  if (Math.abs(a1 - a0) < 0.5) return "";
  const lo = Math.min(a0, a1), hi = Math.max(a0, a1);
  const [x0, y0] = polar(r, lo);
  const [x1, y1] = polar(r, hi);
  return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${r} ${r} 0 ${hi - lo > 180 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
}

class Knob {
  constructor({ label, min = 0, max = 1, value = 0, def = value, bipolar = false, small = false, title = "", onInput, killable = false }) {
    Object.assign(this, { min, max, def, bipolar, onInput, killable });
    this.value = value;
    this.dragging = false;
    this.lastUser = 0;
    this.el = document.createElement("div");
    this.el.className = "knob" + (small ? " sm" : "");
    if (title) this.el.title = title;
    this.el.innerHTML = `
      <svg viewBox="0 0 40 40">
        <path class="k-track" d="${arcPath(16.5, -135, 135)}" fill="none" stroke-width="3" stroke-linecap="round"/>
        <path class="k-value" fill="none" stroke-width="3" stroke-linecap="round"/>
        <circle class="k-body" cx="20" cy="20" r="11.5" stroke-width="1"/>
        <line class="k-pointer" stroke-width="2.2" stroke-linecap="round"/>
      </svg>
      <label>${label}</label>`;
    this.valuePath = this.el.querySelector(".k-value");
    this.pointer = this.el.querySelector(".k-pointer");
    this.render();

    this.el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      this.el.setPointerCapture(e.pointerId);
      this.dragging = true;
      this.startY = e.clientY;
      this.startV = this.value;
      this.el.classList.add("active");
    });
    this.el.addEventListener("pointermove", (e) => {
      if (!this.dragging) return;
      const sens = e.shiftKey ? 900 : 200;
      this.set(this.startV + (this.startY - e.clientY) / sens * (this.max - this.min), true);
    });
    const end = () => {
      this.dragging = false;
      this.el.classList.remove("active");
    };
    this.el.addEventListener("pointerup", end);
    this.el.addEventListener("pointercancel", end);
    this.el.addEventListener("dblclick", () => this.set(this.def, true));
    this.el.addEventListener("wheel", (e) => {
      e.preventDefault();
      const step = (this.max - this.min) / (e.shiftKey ? 200 : 40);
      this.set(this.value + (e.deltaY < 0 ? step : -step), true);
    }, { passive: false });
  }

  set(v, fromUser) {
    v = clamp(v, this.min, this.max);
    // Gentle snap to the centre detent
    if (fromUser && Math.abs(v - this.def) < (this.max - this.min) * 0.012) v = this.def;
    if (v === this.value && !fromUser) return;
    this.value = v;
    this.render();
    if (fromUser) {
      this.lastUser = performance.now();
      this.onInput && this.onInput(v);
    }
  }

  sync(v) {
    if (this.dragging || performance.now() - this.lastUser < 500) return;
    if (Math.abs(v - this.value) > 1e-4) this.set(v, false);
  }

  render() {
    const t = (this.value - this.min) / (this.max - this.min);
    const ang = -135 + t * 270;
    const from = this.bipolar ? 0 : -135;
    this.valuePath.setAttribute("d", arcPath(16.5, from, ang));
    const [x0, y0] = polar(4, ang);
    const [x1, y1] = polar(10, ang);
    this.pointer.setAttribute("x1", x0); this.pointer.setAttribute("y1", y0);
    this.pointer.setAttribute("x2", x1); this.pointer.setAttribute("y2", y1);
    if (this.killable) this.el.classList.toggle("killed", t < 0.02);
  }
}

class Fader {
  constructor({ orientation = "vertical", min = 0, max = 1, value = 0, def = value, invert = false, cls = "", title = "", onInput }) {
    Object.assign(this, { orientation, min, max, def, invert, onInput });
    this.value = value;
    this.dragging = false;
    this.lastUser = 0;
    this.el = document.createElement("div");
    this.el.className = `fader ${orientation} ${cls}`;
    if (title) this.el.title = title;
    this.el.innerHTML = `<div class="f-track"></div><div class="f-ticks"></div><div class="f-cap"></div>`;
    this.cap = this.el.querySelector(".f-cap");
    this.render();

    this.el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      this.el.setPointerCapture(e.pointerId);
      this.dragging = true;
      const p = this.pointerFrac(e);
      this.offset = e.target === this.cap ? p - this.capFrac() : 0;
      this.set(this.fracToValue(p - this.offset), true);
    });
    this.el.addEventListener("pointermove", (e) => {
      if (!this.dragging) return;
      this.set(this.fracToValue(this.pointerFrac(e) - this.offset), true);
    });
    const end = () => { this.dragging = false; };
    this.el.addEventListener("pointerup", end);
    this.el.addEventListener("pointercancel", end);
    this.el.addEventListener("dblclick", () => this.set(this.def, true));
    this.el.addEventListener("wheel", (e) => {
      e.preventDefault();
      const step = (this.max - this.min) / (e.shiftKey ? 400 : 60);
      const dir = this.orientation === "vertical" && !this.invert ? 1 : (this.orientation === "vertical" ? -1 : 1);
      this.set(this.value + (e.deltaY < 0 ? step : -step) * dir, true);
    }, { passive: false });
  }

  pointerFrac(e) {
    const r = this.el.getBoundingClientRect();
    return this.orientation === "vertical"
      ? (e.clientY - r.top - 8) / Math.max(1, r.height - 16)
      : (e.clientX - r.left - 10) / Math.max(1, r.width - 20);
  }

  /** Fraction from top (vertical) or from left (horizontal) */
  capFrac() {
    const t = (this.value - this.min) / (this.max - this.min);
    if (this.orientation === "horizontal") return t;
    return this.invert ? t : 1 - t;
  }

  fracToValue(p) {
    p = clamp(p, 0, 1);
    const t = this.orientation === "horizontal" ? p : (this.invert ? p : 1 - p);
    return this.min + t * (this.max - this.min);
  }

  set(v, fromUser) {
    v = clamp(v, this.min, this.max);
    if (fromUser && Math.abs(v - this.def) < (this.max - this.min) * 0.008) v = this.def;
    if (v === this.value && !fromUser) return;
    this.value = v;
    this.render();
    if (fromUser) {
      this.lastUser = performance.now();
      this.onInput && this.onInput(v);
    }
  }

  sync(v) {
    if (this.dragging || performance.now() - this.lastUser < 500) return;
    if (Math.abs(v - this.value) > 1e-4) this.set(v, false);
  }

  render() {
    this.place(this.cap, this.capFrac());
  }

  place(el, p) {
    if (this.orientation === "vertical") el.style.top = `calc(8px + (100% - 16px) * ${p.toFixed(4)})`;
    else el.style.left = `calc(10px + (100% - 20px) * ${p.toFixed(4)})`;
  }

  /** Marks the position the fader has to reach to take over again (null hides it) */
  setGhost(v) {
    if (v === null || v === undefined) {
      if (this.ghost) this.ghost.hidden = true;
      return;
    }
    if (!this.ghost) {
      this.ghost = document.createElement("div");
      this.ghost.className = "f-ghost";
      this.el.appendChild(this.ghost);
    }
    this.ghost.hidden = false;
    const t = (clamp(v, this.min, this.max) - this.min) / (this.max - this.min);
    this.place(this.ghost, this.orientation === "horizontal" || this.invert ? t : 1 - t);
  }
}

class VuMeter {
  constructor() {
    this.el = document.createElement("div");
    this.el.className = "vu";
    this.el.innerHTML = `<div class="vu-cover"></div><div class="vu-peak"></div>`;
    this.cover = this.el.firstChild;
    this.peakEl = this.el.lastChild;
    this.peak = 0;
    this.peakAt = 0;
    this.last = -1;
  }

  /** linear amplitude -> -36 dB .. +3 dB */
  set(amp) {
    const db = amp > 1e-5 ? 20 * Math.log10(amp) : -100;
    const level = clamp((db + 36) / 39, 0, 1);
    const now = performance.now();
    if (level >= this.peak || now - this.peakAt > 1200) {
      this.peak = level;
      this.peakAt = now;
    }
    const q = Math.round(level * 200) / 200;
    if (q !== this.last) {
      this.cover.style.height = `${(1 - q) * 100}%`;
      this.last = q;
    }
    this.peakEl.style.bottom = `calc(${(this.peak * 100).toFixed(1)}% - 2px)`;
    this.peakEl.style.opacity = this.peak > 0.02 ? ".85" : "0";
  }
}

// ======================================================================
// Decks
// ======================================================================

const D = [null, null];

const PAD_MODES = {
  hotcue: {
    tab: "HOT CUE",
    pad(i, k, d) {
      const t = d && d.hot_cues_seconds[k];
      const set = t !== null && t !== undefined;
      return { main: String(k + 1), sub: set ? fmtTime(t) : "", cls: set ? "set" : "", color: PAD_COLORS[k] };
    },
    press(i, k, e) {
      if (e.shiftKey || e.button === 2) send({ action: "DeleteHotCue", deck: i, pad_index: k });
      else send({ action: "HotCue", deck: i, pad_index: k, pressed: true });
    },
    release(i, k) { send({ action: "HotCue", deck: i, pad_index: k, pressed: false }); },
  },
  loop: {
    tab: "LOOP",
    sizes: [0.25, 0.5, 1, 2, 4, 8, 16, 32],
    pad(i, k, d) {
      const size = this.sizes[k];
      const on = d && d.loop_active && Math.abs(d.loop_size_beats - size) < 1e-3;
      return { main: size < 1 ? `1/${1 / size}` : String(size), sub: "tiempos", cls: on ? "on" : "", color: "var(--loop)" };
    },
    press(i, k) { send({ action: "SetLoop", deck: i, beats: this.sizes[k] }); },
  },
  jump: {
    tab: "SALTO",
    beats: [-1, -2, -4, -8, 1, 2, 4, 8],
    pad(i, k) {
      const b = this.beats[k];
      return { main: b < 0 ? `◀ ${-b}` : `${b} ▶`, sub: Math.abs(b) >= 4 ? `${Math.abs(b) / 4} compás` : "tiempo", cls: "", color: "var(--deck-color)" };
    },
    press(i, k) { send({ action: "BeatJump", deck: i, beats: this.beats[k] }); },
  },
  stems: {
    tab: "STEMS",
    pad(i, k, d) {
      const names = ["VOZ", "BATERÍA", "BAJO"];
      const keys = ["stem_vocal", "stem_drums", "stem_bass"];
      if (!d) return { main: "", sub: "", cls: "disabled" };
      if (k < 3) {
        const on = d[keys[k]] > 0.5;
        return { main: names[k], sub: on ? "activo" : "silenciado", cls: on ? "set" : "off-state", color: ["#ff5fb2", "#f5c524", "#2e7bff"][k] };
      }
      if (k === 3) return { main: "TODO", sub: "restaurar", cls: "", color: "var(--deck-color)" };
      if (k < 7) {
        const solo = keys.every((key, j) => (j === k - 4) === (d[key] > 0.5));
        return { main: `SOLO ${names[k - 4]}`, sub: "", cls: solo ? "on" : "", color: ["#ff5fb2", "#f5c524", "#2e7bff"][k - 4] };
      }
      return { main: "", sub: "", cls: "disabled" };
    },
    press(i, k) {
      const d = deckState(i);
      if (!d) return;
      const stems = ["Vocal", "Drums", "Bass"];
      const keys = ["stem_vocal", "stem_drums", "stem_bass"];
      if (k < 3) send({ action: "SetStemGain", deck: i, stem: stems[k], gain: d[keys[k]] > 0.5 ? 0 : 1 });
      else if (k === 3) stems.forEach((s) => send({ action: "SetStemGain", deck: i, stem: s, gain: 1 }));
      else if (k < 7) stems.forEach((s, j) => send({ action: "SetStemGain", deck: i, stem: s, gain: j === k - 4 ? 1 : 0 }));
    },
  },
  fx: {
    tab: "ECHO",
    beats: [null, 0.25, 0.5, 0.75, 1, 2],
    pad(i, k, d) {
      if (!d) return { main: "", sub: "", cls: "disabled" };
      if (k === 0) return { main: "ECHO", sub: d.echo_active ? "ON" : "OFF", cls: d.echo_active ? "on" : "", color: "var(--deck-color)" };
      if (k < 6) {
        const b = this.beats[k];
        const on = Math.abs(d.echo_beats - b) < 1e-3;
        return { main: b < 1 ? `${b === 0.75 ? "3/4" : `1/${1 / b}`}` : String(b), sub: "tiempo", cls: on ? "set" : "", color: "var(--deck-color)" };
      }
      return { main: k === 6 ? "WET −" : "WET +", sub: `${Math.round(d.echo_wet * 100)}%`, cls: "", color: "var(--deck-color)" };
    },
    press(i, k) {
      const d = deckState(i);
      if (!d) return;
      const cmd = { action: "SetEchoFx", deck: i, active: d.echo_active, wet: d.echo_wet, beats: d.echo_beats };
      if (k === 0) cmd.active = !d.echo_active;
      else if (k < 6) { cmd.beats = this.beats[k]; cmd.active = true; }
      else cmd.wet = clamp(d.echo_wet + (k === 6 ? -0.1 : 0.1), 0, 1);
      send(cmd);
    },
  },
};

function buildDeck(i) {
  const root = $(`#deck-${i}`);
  root.innerHTML = `
    <div class="deck-head">
      <div class="deck-badge">${DECK_NAME[i]}</div>
      <div class="track-info">
        <div class="track-title" data-ref="title"></div>
        <div class="track-artist" data-ref="artist"></div>
      </div>
      <div class="key-chip unknown" data-ref="key" title="Tonalidad (Camelot)">—</div>
      <div class="tempo-readout">
        <div class="bpm-big" data-ref="bpm">—</div>
        <div class="pitch-small" data-ref="pitch">±0.00%</div>
      </div>
      <div class="time-readout" data-ref="time" title="Clic: tiempo transcurrido / restante">
        <div class="time-main" data-ref="timeMain">00:00.0</div>
        <div class="time-sub" data-ref="timeSub">00:00.0</div>
      </div>
    </div>
    <div class="overview" data-ref="overview" title="Clic para saltar (con Shift mientras suena)">
      <canvas></canvas>
      <div class="overview-hint" data-ref="ovHint">Arrastra una pista aquí o elige una en la biblioteca</div>
    </div>
    <div class="deck-body">
      <div class="tempo-col">
        <button class="chip-btn" data-ref="range" title="Rango del pitch">±8%</button>
        <div data-ref="tempoSlot" style="flex:1;display:flex;justify-content:center;min-height:0"></div>
        <div class="tempo-value" data-ref="tempoVal">0.00%</div>
        <button class="chip-btn" data-ref="zero" title="Fija el tempo actual como nuevo 0% sin cambiar la velocidad (estilo Serato)">0%</button>
      </div>
      <div class="deck-main">
        <div class="transport">
          <button class="round-btn btn-cue" data-ref="cue" title="CUE">CUE</button>
          <button class="round-btn btn-play" data-ref="play" title="Play / Pausa">${ICON.play}</button>
          <button class="chip-btn" data-ref="sync" title="Beat sync con el otro deck">SYNC</button>
          <span class="master-badge" data-ref="master">MASTER</span>
          <div class="grow"></div>
          <button class="chip-btn" data-ref="jumpBack" title="Saltar 1 compás atrás">◀ 4</button>
          <button class="chip-btn" data-ref="jumpFwd" title="Saltar 1 compás adelante">4 ▶</button>
        </div>
        <div class="pad-tabs" data-ref="tabs">
          ${Object.entries(PAD_MODES).map(([k, m]) => `<button class="pad-tab" data-mode="${k}">${m.tab}</button>`).join("")}
        </div>
        <div class="pads" data-ref="pads">
          ${Array.from({ length: 8 }, (_, k) => `<button class="pad" data-k="${k}"><span></span><small></small></button>`).join("")}
        </div>
        <div class="loop-strip">
          <button class="chip-btn" data-ref="loopIn" title="Loop In">IN</button>
          <button class="chip-btn" data-ref="loopOut" title="Loop Out">OUT</button>
          <button class="chip-btn" data-ref="reloop" title="Reloop / Salir del loop">RELOOP</button>
          <div class="grow"></div>
          <button class="chip-btn" data-ref="loopHalf" title="Mitad del loop">÷2</button>
          <button class="loop-size" data-ref="loopSize" title="Auto loop cuantizado">4</button>
          <button class="chip-btn" data-ref="loopDouble" title="Doble del loop">×2</button>
          <div class="grow"></div>
          <button class="chip-btn slip" data-ref="slip" title="Modo Slip: la pista sigue avanzando en segundo plano">SLIP</button>
        </div>
      </div>
    </div>`;

  const r = {};
  $$("[data-ref]", root).forEach((el) => { r[el.dataset.ref] = el; });
  r.root = root;
  r.ovCanvas = $("canvas", r.overview);
  r.padEls = $$(".pad", r.pads);
  r.waveRow = $(`.wave-row[data-deck="${i}"]`);
  r.waveCanvas = $("canvas", r.waveRow);
  r.counter = $('[data-bind="counter"]', r.waveRow);
  r.dots = $$('[data-bind="dots"] i', r.waveRow);

  r.tempo = new Fader({
    min: -1, max: 1, value: 0, def: 0, invert: true, cls: "tempo",
    title: "Tempo (doble clic = 0)",
    onInput: (v) => send({ action: "SetTempo", deck: i, value: v }),
  });
  r.tempo.el.insertAdjacentHTML("beforeend", '<div class="f-center"></div><span class="f-label top">−</span><span class="f-label bottom">+</span>');
  r.tempoSlot.appendChild(r.tempo.el);

  // Transport
  const cue = r.cue;
  cue.addEventListener("pointerdown", (e) => { cue.setPointerCapture(e.pointerId); send({ action: "Cue", deck: i, pressed: true }); });
  const cueUp = () => send({ action: "Cue", deck: i, pressed: false });
  cue.addEventListener("pointerup", cueUp);
  cue.addEventListener("pointercancel", cueUp);
  r.play.addEventListener("click", () => send({ action: "PlayPause", deck: i }));
  r.sync.addEventListener("click", () => send({ action: "Sync", deck: i }));
  r.zero.addEventListener("click", () => send({ action: "ReassignMasterTempo", deck: i }));
  r.range.addEventListener("click", () => {
    const ranges = [0.08, 0.16, 0.5];
    const d = deckState(i);
    const cur = d ? d.pitch_range : 0.08;
    const next = ranges[(ranges.findIndex((x) => Math.abs(x - cur) < 1e-3) + 1) % ranges.length];
    send({ action: "SetPitchRange", deck: i, range: next });
  });
  r.jumpBack.addEventListener("click", () => send({ action: "BeatJump", deck: i, beats: -4 }));
  r.jumpFwd.addEventListener("click", () => send({ action: "BeatJump", deck: i, beats: 4 }));
  r.time.addEventListener("click", () => {
    S.timeMode[i] = S.timeMode[i] === "elapsed" ? "remaining" : "elapsed";
    store.set("timeMode", S.timeMode);
  });

  // Loops
  r.loopIn.addEventListener("click", () => send({ action: "LoopIn", deck: i }));
  r.loopOut.addEventListener("click", () => send({ action: "LoopOut", deck: i }));
  r.reloop.addEventListener("click", () => send({ action: "ReloopExit", deck: i }));
  r.loopHalf.addEventListener("click", () => send({ action: "LoopResize", deck: i, factor: 0.5 }));
  r.loopDouble.addEventListener("click", () => send({ action: "LoopResize", deck: i, factor: 2 }));
  r.loopSize.addEventListener("click", () => {
    const d = deckState(i);
    send({ action: "SetLoop", deck: i, beats: d ? d.loop_size_beats : 4 });
  });
  r.slip.addEventListener("click", () => send({ action: "ToggleSlip", deck: i }));

  // Pads
  r.tabs.addEventListener("click", (e) => {
    const b = e.target.closest(".pad-tab");
    if (!b) return;
    S.padMode[i] = b.dataset.mode;
    store.set("padMode", S.padMode);
    updatePads(i, true);
  });
  r.pads.addEventListener("contextmenu", (e) => e.preventDefault());
  r.pads.addEventListener("pointerdown", (e) => {
    const pad = e.target.closest(".pad");
    if (!pad) return;
    pad.setPointerCapture(e.pointerId);
    const mode = PAD_MODES[S.padMode[i]];
    mode.press(i, +pad.dataset.k, e);
  });
  r.pads.addEventListener("pointerup", (e) => {
    const pad = e.target.closest(".pad");
    if (!pad) return;
    const mode = PAD_MODES[S.padMode[i]];
    mode.release && mode.release(i, +pad.dataset.k);
  });

  // Overview seek
  r.overview.addEventListener("pointerdown", (e) => {
    const d = deckState(i);
    const t = S.tracks[i];
    if (!d || !t || !d.loaded) return;
    if (d.is_playing && !e.shiftKey) {
      if (!S.seekHintShown) toast("Mantén Shift para saltar mientras el deck está sonando.", "warn");
      S.seekHintShown = true;
      return;
    }
    const rect = r.overview.getBoundingClientRect();
    const frac = clamp((e.clientX - rect.left) / rect.width, 0, 1);
    send({ action: "Seek", deck: i, position: frac * d.duration_seconds });
  });

  // Waveform: drag to scrub (paused) or nudge (playing), wheel = zoom
  const wc = r.waveCanvas;
  let dragX = null;
  wc.addEventListener("pointerdown", (e) => {
    if (!S.tracks[i]) return;
    wc.setPointerCapture(e.pointerId);
    dragX = e.clientX;
    r.scrubPos = estPos(i);
  });
  wc.addEventListener("pointermove", (e) => {
    if (dragX === null) return;
    const d = deckState(i);
    const dx = e.clientX - dragX;
    dragX = e.clientX;
    if (!d) return;
    if (d.is_playing) {
      send({ action: "Jog", deck: i, delta: -dx * 0.6 });
    } else {
      r.scrubPos = clamp(r.scrubPos - dx / ZOOMS[S.zoomIdx], 0, d.duration_seconds);
      send({ action: "Seek", deck: i, position: r.scrubPos });
    }
  });
  const endDrag = () => { dragX = null; };
  wc.addEventListener("pointerup", endDrag);
  wc.addEventListener("pointercancel", endDrag);
  wc.addEventListener("wheel", (e) => {
    e.preventDefault();
    setZoom(S.zoomIdx + (e.deltaY < 0 ? 1 : -1));
  }, { passive: false });

  // Drop targets
  for (const target of [root, r.waveRow]) setupDropTarget(target, i);

  D[i] = r;
  updatePads(i, true);
  updateDeckHeader(i);
}

function setupDropTarget(el, deck) {
  let depth = 0;
  el.addEventListener("dragenter", (e) => {
    e.preventDefault();
    depth++;
    D[deck].root.classList.add("drop-target");
    D[deck].waveRow.classList.add("drop-target");
  });
  el.addEventListener("dragover", (e) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  });
  el.addEventListener("dragleave", () => {
    depth = Math.max(0, depth - 1);
    if (depth === 0) {
      D[deck].root.classList.remove("drop-target");
      D[deck].waveRow.classList.remove("drop-target");
    }
  });
  el.addEventListener("drop", (e) => {
    e.preventDefault();
    e.stopPropagation();
    depth = 0;
    D[deck].root.classList.remove("drop-target");
    D[deck].waveRow.classList.remove("drop-target");
    S.shift = e.shiftKey;
    const libPath = e.dataTransfer.getData("application/x-bubba-path");
    if (libPath) loadPath(deck, libPath);
    else if (e.dataTransfer.files.length) uploadFile(deck, e.dataTransfer.files[0]);
  });
}

function updateDeckHeader(i) {
  const r = D[i];
  if (!r) return;
  const t = S.tracks[i];
  const loading = S.loading[i];
  r.root.classList.toggle("empty", !t);
  if (loading) {
    setText(r.title, `Analizando ${loading}…`);
    setText(r.artist, "Detectando BPM, rejilla y tonalidad");
  } else if (t) {
    setText(r.title, t.title || "Sin título");
    setText(r.artist, (t.artist || "Artista desconocido") + (t.refining ? " · afinando rejilla…" : ""));
  } else {
    setText(r.title, "Deck vacío");
    setText(r.artist, "Arrastra una pista o cárgala desde la biblioteca");
  }
  const key = t ? t.camelot : null;
  const color = camelotColor(key);
  r.key.classList.toggle("unknown", !color);
  r.key.style.setProperty("--key-color", color || "");
  setText(r.key, key && color ? key : "—");
  r.key.title = t && t.key && t.key !== "--" ? `Tonalidad: ${t.key}` : "Tonalidad desconocida";
  r.ovHint.classList.toggle("hidden", !!t);

  let bar = r.overview.querySelector(".loading-bar");
  if (loading && !bar) r.overview.insertAdjacentHTML("beforeend", '<div class="loading-bar"></div>');
  if (!loading && bar) bar.remove();
}

function updatePads(i, force) {
  const r = D[i];
  const mode = PAD_MODES[S.padMode[i]] || PAD_MODES.hotcue;
  if (force) $$(".pad-tab", r.tabs).forEach((b) => b.classList.toggle("on", b.dataset.mode === S.padMode[i]));
  const d = deckState(i);
  r.padEls.forEach((el, k) => {
    const p = mode.pad(i, k, d);
    const key = `${p.main}|${p.sub}|${p.cls}|${p.color}`;
    if (!force && el._k === key) return;
    el._k = key;
    el.firstChild.textContent = p.main;
    el.lastChild.textContent = p.sub;
    el.className = `pad ${p.cls}`;
    el.style.setProperty("--pad-color", p.color || "var(--deck-color)");
  });
}

function updateDeck(i, d) {
  const r = D[i];
  r.root.classList.toggle("is-playing", d.is_playing);

  const bpm = d.loaded ? d.effective_bpm : null;
  const bpmTxt = bpm ? bpm.toFixed(2) : "—";
  if (r.bpm._t !== bpmTxt) {
    r.bpm._t = bpmTxt;
    if (bpm) {
      const [a, b] = bpmTxt.split(".");
      r.bpm.innerHTML = `${a}<small>.${b}</small>`;
    } else r.bpm.textContent = "—";
  }
  const pct = d.pitch_percent;
  setText(r.pitch, fmtPct(pct));
  r.pitch.classList.toggle("up", pct > 0.005);
  r.pitch.classList.toggle("down", pct < -0.005);
  setText(r.tempoVal, fmtPct(pct));
  setText(r.range, `±${Math.round(d.pitch_range * 100)}%`);
  r.tempo.sync(d.tempo_fader);
  r.tempo.setGhost(d.tempo_pickup);
  const tempoTip = d.tempo_pickup !== null
    ? "Lleva el fader a la marca naranja para retomar el tempo"
    : "Tempo (doble clic = 0)";
  if (r.tempo.el.title !== tempoTip) r.tempo.el.title = tempoTip;

  r.play.classList.toggle("lit", d.is_playing);
  const playIcon = d.is_playing ? "pause" : "play";
  if (r.play._i !== playIcon) { r.play.innerHTML = ICON[playIcon]; r.play._i = playIcon; }
  r.cue.classList.toggle("lit", d.loaded && !d.is_playing && Math.abs(d.position_seconds - d.cue_position_seconds) < 0.01);
  r.sync.classList.toggle("on", d.is_sync_enabled);
  r.master.classList.toggle("on", d.is_master);
  r.slip.classList.toggle("on", d.is_slip_active);
  r.loopIn.classList.toggle("on", d.loop_in_seconds !== null);
  r.reloop.classList.toggle("loop", true);
  r.reloop.classList.toggle("on", d.loop_active);
  r.loopSize.classList.toggle("on", d.loop_active);
  const ls = d.loop_size_beats;
  setText(r.loopSize, ls < 1 ? `1/${Math.round(1 / ls)}` : String(+ls.toFixed(2)));

  updatePads(i, false);
}

// ======================================================================
// Mixer
// ======================================================================

const M = { ch: [null, null] };

function buildMixer() {
  const root = $("#mixer");
  root.innerHTML = "";

  const makeChannel = (i) => {
    const ch = document.createElement("div");
    ch.className = "channel";
    ch.dataset.deck = i;
    const knobs = document.createElement("div");
    knobs.className = "channel-knobs";
    const c = {};
    c.trim = new Knob({ label: "TRIM", value: 0.5, def: 0.5, small: true, bipolar: true, title: "Ganancia de entrada", onInput: (v) => send({ action: "SetTrim", deck: i, value: v }) });
    c.hi = new Knob({ label: "HI", value: 0.5, def: 0.5, small: true, bipolar: true, killable: true, onInput: (v) => send({ action: "SetEq", deck: i, band: "High", value: v }) });
    c.mid = new Knob({ label: "MID", value: 0.5, def: 0.5, small: true, bipolar: true, killable: true, onInput: (v) => send({ action: "SetEq", deck: i, band: "Mid", value: v }) });
    c.low = new Knob({ label: "LOW", value: 0.5, def: 0.5, small: true, bipolar: true, killable: true, onInput: (v) => send({ action: "SetEq", deck: i, band: "Low", value: v }) });
    c.cfx = new Knob({ label: "FILTER", min: -1, max: 1, value: 0, def: 0, small: true, bipolar: true, title: "Color FX: izquierda = pasa-bajos, derecha = pasa-altos", onInput: (v) => send({ action: "SetColorFx", deck: i, value: v }) });
    [c.trim, c.hi, c.mid, c.low, c.cfx].forEach((k) => knobs.appendChild(k.el));

    const faderCol = document.createElement("div");
    faderCol.className = "channel-fader";
    c.cue = document.createElement("button");
    c.cue.className = "ch-cue";
    c.cue.title = "Pre-escucha en audífonos (CUE)";
    c.cue.innerHTML = ICON.headphones;
    c.cue.addEventListener("click", () => send({ action: "HeadphoneCue", deck: i }));
    const fm = document.createElement("div");
    fm.className = "fader-meter";
    c.vu = new VuMeter();
    c.fader = new Fader({ value: 1, def: 1, title: "Volumen del canal", onInput: (v) => send({ action: "SetVolume", deck: i, volume: v }) });
    if (i === 0) { fm.appendChild(c.fader.el); fm.appendChild(c.vu.el); }
    else { fm.appendChild(c.vu.el); fm.appendChild(c.fader.el); }
    faderCol.appendChild(c.cue);
    faderCol.appendChild(fm);

    ch.appendChild(knobs);
    ch.appendChild(faderCol);
    M.ch[i] = c;
    return ch;
  };

  root.appendChild(makeChannel(0));

  const center = document.createElement("div");
  center.className = "mixer-center";
  center.innerHTML = `<span class="mixer-label">MASTER</span>`;
  M.master = new Knob({ label: "NIVEL", value: 0.8, def: 0.8, small: true, title: "Volumen master", onInput: (v) => send({ action: "SetMasterVolume", value: v }) });
  center.appendChild(M.master.el);
  const mm = document.createElement("div");
  mm.className = "master-meters";
  M.vuL = new VuMeter();
  M.vuR = new VuMeter();
  mm.appendChild(M.vuL.el);
  mm.appendChild(M.vuR.el);
  center.appendChild(mm);
  center.insertAdjacentHTML("beforeend", `<span class="mixer-label">AUDÍFONOS</span>`);
  M.hpMix = new Knob({ label: "CUE·MST", value: 0, def: 0.5, small: true, title: "Mezcla en audífonos: pre-escucha ↔ master", onInput: (v) => send({ action: "SetHeadphoneMix", mix: v }) });
  M.hpVol = new Knob({ label: "NIVEL", value: 0.8, def: 0.8, small: true, title: "Volumen de audífonos", onInput: (v) => send({ action: "SetHeadphoneVolume", volume: v }) });
  center.appendChild(M.hpMix.el);
  center.appendChild(M.hpVol.el);
  root.appendChild(center);

  root.appendChild(makeChannel(1));

  const xf = document.createElement("div");
  xf.className = "xfader-row";
  xf.innerHTML = `<span class="xf-label" style="color:var(--accent-a)">1</span>`;
  M.xfader = new Fader({ orientation: "horizontal", min: -1, max: 1, value: 0, def: 0, title: "Crossfader (doble clic = centro)", onInput: (v) => send({ action: "SetCrossfader", value: v }) });
  xf.appendChild(M.xfader.el);
  xf.insertAdjacentHTML("beforeend", `<span class="xf-label" style="color:var(--accent-b)">2</span>`);
  root.appendChild(xf);
}

function updateMixer(m) {
  [m.deck1, m.deck2].forEach((d, i) => {
    const c = M.ch[i];
    c.trim.sync(d.trim);
    c.hi.sync(d.eq_high);
    c.mid.sync(d.eq_mid);
    c.low.sync(d.eq_low);
    c.cfx.sync(d.color_fx);
    c.fader.sync(d.volume);
    c.cue.classList.toggle("on", d.headphone_cue);
    c.vu.set(Math.max(d.vu_l, d.vu_r));
  });
  M.master.sync(m.master_volume);
  M.hpMix.sync(m.headphone_mix);
  M.hpVol.sync(m.headphone_volume);
  M.xfader.sync(m.crossfader);
  M.vuL.set(m.master_vu_l);
  M.vuR.set(m.master_vu_r);
}

function updateFromState() {
  const m = S.mixer;
  updateDeck(0, m.deck1);
  updateDeck(1, m.deck2);
  updateMixer(m);

  const master = m.deck1.is_master ? m.deck1 : m.deck2.is_master ? m.deck2 : (m.deck1.is_playing ? m.deck1 : m.deck2.is_playing ? m.deck2 : null);
  setText($("#master-bpm"), master && master.loaded ? master.effective_bpm.toFixed(2) : "—");
  $$("#sync-mode button").forEach((b) => b.classList.toggle("on", b.dataset.mode === m.sync_mode));
}

function updateKeyCompat() {
  const el = $("#key-compat");
  const a = S.tracks[0], b = S.tracks[1];
  const rel = a && b ? keyRelation(a.camelot, b.camelot) : null;
  el.classList.toggle("good", !!(rel && rel.ok));
  el.classList.toggle("bad", !!(rel && !rel.ok));
  el.textContent = rel ? `${a.camelot} · ${b.camelot}  ${rel.ok ? "✓" : "✕"} ${rel.text}` : "Tonalidad —";
}

// ======================================================================
// Rendering (waveforms, overview, beat counters, phase meter)
// ======================================================================

function fitCanvas(canvas) {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(canvas.clientWidth * dpr));
  const h = Math.max(1, Math.round(canvas.clientHeight * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
    return true;
  }
  return false;
}

function bandsAt(frames, n, f0, f1) {
  let lo = 0, mi = 0, hi = 0;
  if (f1 <= f0) f1 = f0 + 1;
  for (let f = Math.max(0, f0); f < Math.min(n, f1); f++) {
    const o = f * 4;
    if (frames[o] > lo) lo = frames[o];
    if (frames[o + 1] > mi) mi = frames[o + 1];
    if (frames[o + 2] > hi) hi = frames[o + 2];
  }
  return [lo, mi, hi];
}

const wfScale = (v) => Math.pow(v / 255, 0.8);

function drawWave(i) {
  const r = D[i];
  const c = r.waveCanvas;
  fitCanvas(c);
  const ctx = c.getContext("2d");
  const w = c.width, h = c.height;
  const dpr = window.devicePixelRatio || 1;
  const deckColor = i === 0 ? "#2e9bff" : "#ff7b2e";
  ctx.fillStyle = "#07080a";
  ctx.fillRect(0, 0, w, h);

  const mid = h / 2;
  const t = S.tracks[i];
  const d = deckState(i);
  const playX = Math.round(w * 0.5);

  ctx.fillStyle = "#15181d";
  ctx.fillRect(0, Math.round(mid), w, 1);

  if (!t || !d || !d.loaded) {
    ctx.fillStyle = "#3a404a";
    ctx.font = `${12 * dpr}px "Segoe UI", sans-serif`;
    ctx.textAlign = "center";
    ctx.fillText(S.loading[i] ? "Analizando…" : `Deck ${DECK_NAME[i]} vacío`, w / 2, mid - 8 * dpr);
    ctx.textAlign = "start";
    return;
  }

  const pos = estPos(i);
  const pps = ZOOMS[S.zoomIdx] * dpr;
  const xOf = (sec) => playX + (sec - pos) * pps;
  const tStart = pos - playX / pps;
  const tEnd = pos + (w - playX) / pps;

  // Loop region
  if (d.loop_active) {
    const x0 = xOf(d.loop_start_seconds), x1 = xOf(d.loop_end_seconds);
    ctx.fillStyle = "rgba(43,212,107,.13)";
    ctx.fillRect(x0, 0, x1 - x0, h);
  }

  // 3-band waveform
  const frames = t.frames, n = frames.length / 4, fps = t.fps;
  const amp = h * 0.47;
  const pLow = new Path2D(), pMid = new Path2D(), pHigh = new Path2D();
  for (let x = 0; x < w; x++) {
    const sec = tStart + x / pps;
    if (sec < 0 || sec > t.duration) continue;
    const f0 = Math.floor(sec * fps);
    const f1 = Math.floor((sec + 1 / pps) * fps);
    const [lo, mi, hi] = bandsAt(frames, n, f0, f1);
    const hl = wfScale(lo) * amp, hm = wfScale(mi) * amp * 0.78, hh = wfScale(hi) * amp * 0.55;
    if (hl > 0.5) pLow.rect(x, mid - hl, 1, hl * 2);
    if (hm > 0.5) pMid.rect(x, mid - hm, 1, hm * 2);
    if (hh > 0.5) pHigh.rect(x, mid - hh, 1, hh * 2);
  }
  ctx.fillStyle = "#2e6dff"; ctx.fill(pLow);
  ctx.fillStyle = "#ffa62e"; ctx.fill(pMid);
  ctx.fillStyle = "#f4f6fb"; ctx.fill(pHigh);

  // Played part slightly dimmed
  ctx.fillStyle = "rgba(7,8,10,.42)";
  ctx.fillRect(0, 0, playX, h);

  // Beat grid
  const g = t.grid;
  if (g && g.beat_interval_seconds > 0) {
    const iv = g.beat_interval_seconds, first = g.first_downbeat_seconds;
    const k0 = Math.ceil((tStart - first) / iv), k1 = Math.floor((tEnd - first) / iv);
    const spacing = iv * pps;
    for (let k = k0; k <= k1; k++) {
      const down = ((k % 4) + 4) % 4 === 0;
      if (!down && spacing < 7 * dpr) continue;
      const x = Math.round(xOf(first + k * iv));
      if (down) {
        ctx.fillStyle = "rgba(255,255,255,.42)";
        ctx.fillRect(x, 0, Math.max(1, dpr), h);
        ctx.fillStyle = "rgba(255,255,255,.85)";
        ctx.fillRect(x - 2 * dpr, 0, 5 * dpr, 2 * dpr);
        ctx.fillRect(x - 2 * dpr, h - 2 * dpr, 5 * dpr, 2 * dpr);
      } else {
        ctx.fillStyle = "rgba(255,255,255,.13)";
        ctx.fillRect(x, 0, Math.max(1, dpr * 0.75), h);
      }
    }
  }

  // Loop borders + pending loop-in
  if (d.loop_active) {
    ctx.fillStyle = "#2bd46b";
    ctx.fillRect(Math.round(xOf(d.loop_start_seconds)), 0, 2 * dpr, h);
    ctx.fillRect(Math.round(xOf(d.loop_end_seconds)) - 2 * dpr, 0, 2 * dpr, h);
  }
  if (d.loop_in_seconds !== null) {
    ctx.fillStyle = "rgba(43,212,107,.8)";
    ctx.fillRect(Math.round(xOf(d.loop_in_seconds)), 0, 2 * dpr, h);
  }

  // Cue point
  const cx = xOf(d.cue_position_seconds);
  if (cx > -10 && cx < w + 10) {
    ctx.fillStyle = "#f5a524";
    ctx.beginPath();
    ctx.moveTo(cx - 6 * dpr, 0); ctx.lineTo(cx + 6 * dpr, 0); ctx.lineTo(cx, 8 * dpr); ctx.closePath();
    ctx.fill();
    ctx.fillRect(cx - 0.5 * dpr, 0, dpr, h);
  }

  // Hot cues
  ctx.font = `bold ${10 * dpr}px "Segoe UI", sans-serif`;
  d.hot_cues_seconds.forEach((sec, k) => {
    if (sec === null) return;
    const x = xOf(sec);
    if (x < -20 || x > w + 2) return;
    ctx.fillStyle = PAD_COLORS[k];
    ctx.fillRect(Math.round(x), 0, Math.max(1, dpr), h);
    ctx.fillRect(Math.round(x), h - 14 * dpr, 13 * dpr, 14 * dpr);
    ctx.fillStyle = "#0b0c0f";
    ctx.fillText(String(k + 1), x + 3.5 * dpr, h - 3.5 * dpr);
  });

  // Suggested cues (phrase analysis): flag at the top
  (t.cues || []).forEach((c) => {
    const x = Math.round(xOf(c.seconds));
    if (x < -60 || x > w + 2) return;
    const st = CUE_STYLE[c.kind] || CUE_STYLE.Start;
    ctx.fillStyle = st.color;
    ctx.globalAlpha = 0.45;
    ctx.fillRect(x, 0, Math.max(1, dpr), h);
    ctx.globalAlpha = 1;
    ctx.fillRect(x, 0, ctx.measureText(st.label).width + 6 * dpr, 13 * dpr);
    ctx.fillStyle = "#0b0c0f";
    ctx.fillText(st.label, x + 3 * dpr, 10 * dpr);
  });

  // Slip ghost playhead
  if (d.is_slip_active && Math.abs(d.slip_position_seconds - d.position_seconds) > 0.05) {
    const sx = Math.round(xOf(d.slip_position_seconds));
    ctx.fillStyle = "rgba(176,124,255,.9)";
    for (let y = 0; y < h; y += 6 * dpr) ctx.fillRect(sx, y, 2 * dpr, 3 * dpr);
  }

  // Playhead
  ctx.fillStyle = deckColor;
  ctx.globalAlpha = 0.35;
  ctx.fillRect(playX - 3 * dpr, 0, 6 * dpr, h);
  ctx.globalAlpha = 1;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(playX - dpr, 0, 2 * dpr, h);
  ctx.beginPath();
  ctx.moveTo(playX - 5 * dpr, 0); ctx.lineTo(playX + 5 * dpr, 0); ctx.lineTo(playX, 5 * dpr); ctx.closePath();
  ctx.moveTo(playX - 5 * dpr, h); ctx.lineTo(playX + 5 * dpr, h); ctx.lineTo(playX, h - 5 * dpr); ctx.closePath();
  ctx.fill();
}

function buildOverview(i, w, h) {
  const t = S.tracks[i];
  const off = document.createElement("canvas");
  off.width = w; off.height = h;
  const ctx = off.getContext("2d");
  const frames = t.frames, n = frames.length / 4;
  const mid = h / 2, amp = h * 0.46;
  const pLow = new Path2D(), pMid = new Path2D(), pHigh = new Path2D();
  for (let x = 0; x < w; x++) {
    const f0 = Math.floor(x / w * n), f1 = Math.floor((x + 1) / w * n);
    const [lo, mi, hi] = bandsAt(frames, n, f0, f1);
    const hl = wfScale(lo) * amp, hm = wfScale(mi) * amp * 0.8, hh = wfScale(hi) * amp * 0.55;
    pLow.rect(x, mid - hl, 1, hl * 2);
    pMid.rect(x, mid - hm, 1, hm * 2);
    pHigh.rect(x, mid - hh, 1, hh * 2);
  }
  ctx.fillStyle = "#2e6dff"; ctx.fill(pLow);
  ctx.fillStyle = "#ffa62e"; ctx.fill(pMid);
  ctx.fillStyle = "#f4f6fb"; ctx.fill(pHigh);
  t.overview = { canvas: off, w, h };
}

function drawOverview(i) {
  const r = D[i];
  const c = r.ovCanvas;
  fitCanvas(c);
  const ctx = c.getContext("2d");
  const w = c.width, h = c.height;
  const dpr = window.devicePixelRatio || 1;
  ctx.clearRect(0, 0, w, h);
  const t = S.tracks[i];
  const d = deckState(i);
  if (!t || !d || !d.loaded || !d.duration_seconds) return;
  if (!t.overview || t.overview.w !== w || t.overview.h !== h) buildOverview(i, w, h);
  ctx.drawImage(t.overview.canvas, 0, 0);

  const dur = d.duration_seconds;
  const xOf = (sec) => sec / dur * w;
  const px = xOf(estPos(i));
  ctx.fillStyle = "rgba(7,8,10,.6)";
  ctx.fillRect(0, 0, px, h);

  if (d.loop_active) {
    ctx.fillStyle = "rgba(43,212,107,.35)";
    ctx.fillRect(xOf(d.loop_start_seconds), 0, Math.max(2, xOf(d.loop_end_seconds) - xOf(d.loop_start_seconds)), h);
  }
  ctx.fillStyle = "#f5a524";
  ctx.fillRect(xOf(d.cue_position_seconds), 0, dpr, h);
  (t.cues || []).forEach((c) => {
    ctx.fillStyle = (CUE_STYLE[c.kind] || CUE_STYLE.Start).color;
    ctx.fillRect(xOf(c.seconds) - dpr, 0, 2 * dpr, 6 * dpr);
  });
  d.hot_cues_seconds.forEach((sec, k) => {
    if (sec === null) return;
    ctx.fillStyle = PAD_COLORS[k];
    ctx.fillRect(xOf(sec) - dpr, h - 5 * dpr, 3 * dpr, 5 * dpr);
  });
  ctx.fillStyle = "#fff";
  ctx.fillRect(Math.round(px) - dpr, 0, 2 * dpr, h);
}

function updateTimes(i) {
  const r = D[i];
  const d = deckState(i);
  if (!d || !d.loaded) {
    setText(r.timeMain, "00:00.0");
    setText(r.timeSub, "00:00.0");
    r.time.classList.remove("warn");
    return;
  }
  const pos = estPos(i);
  const remain = Math.max(0, d.duration_seconds - pos);
  const elapsed = S.timeMode[i] === "elapsed";
  setText(r.timeMain, elapsed ? fmtTime(pos) : `-${fmtTime(remain)}`);
  setText(r.timeSub, elapsed ? `-${fmtTime(remain)}` : fmtTime(pos));
  r.time.classList.toggle("warn", d.is_playing && remain < 30);
}

function beatInfo(i) {
  const t = S.tracks[i];
  const d = deckState(i);
  if (!t || !d || !d.loaded || !t.grid || !(t.grid.beat_interval_seconds > 0)) return null;
  const beats = (estPos(i) - t.grid.first_downbeat_seconds) / t.grid.beat_interval_seconds;
  return { beats, phase: beats - Math.floor(beats) };
}

function updateBeatCounters() {
  const infos = [beatInfo(0), beatInfo(1)];
  infos.forEach((b, i) => {
    const r = D[i];
    if (!b) {
      setText(r.counter, "—");
      r.dots.forEach((dot) => dot.classList.remove("on"));
      return;
    }
    const whole = Math.floor(b.beats);
    const inBar = ((whole % 4) + 4) % 4;
    const bar = Math.floor(whole / 4) + 1;
    setText(r.counter, whole < 0 ? `${whole}` : `${bar}.${inBar + 1}`);
    r.dots.forEach((dot, k) => dot.classList.toggle("on", whole >= 0 && k === inBar));
  });

  const needle = $("#pm-needle");
  const d1 = deckState(0), d2 = deckState(1);
  if (infos[0] && infos[1] && d1.is_playing && d2.is_playing) {
    let diff = infos[1].phase - infos[0].phase;
    if (diff > 0.5) diff -= 1;
    if (diff < -0.5) diff += 1;
    const pct = clamp(diff * 2, -1, 1) * 50;
    needle.style.left = pct >= 0 ? "50%" : `${50 + pct}%`;
    needle.style.width = `${Math.max(1.5, Math.abs(pct))}%`;
    needle.style.background = Math.abs(diff) < 0.03 ? "var(--ok)" : Math.abs(diff) < 0.1 ? "var(--cue)" : "var(--danger)";
  } else {
    needle.style.left = "50%";
    needle.style.width = "0";
  }
}

function frame() {
  if (D[0] && D[1]) {
    for (let i = 0; i < 2; i++) {
      drawWave(i);
      drawOverview(i);
      updateTimes(i);
    }
    updateBeatCounters();
  }
  requestAnimationFrame(frame);
}

function setZoom(idx) {
  S.zoomIdx = clamp(idx, 0, ZOOMS.length - 1);
  store.set("zoom", S.zoomIdx);
}

// ======================================================================
// Library
// ======================================================================

const History = {
  items: store.get("history", []),
  add(msg) {
    this.items = this.items.filter((x) => x.path !== msg.path);
    this.items.unshift({
      path: msg.path, name: msg.path.split(/[\\/]/).pop(), title: msg.title, artist: msg.artist,
      bpm: msg.bpm, camelot: msg.camelot, duration: msg.duration,
      ext: (msg.path.split(".").pop() || "").toUpperCase(),
    });
    this.items = this.items.slice(0, 200);
    store.set("history", this.items);
    if (Lib.mode === "history") Lib.render();
  },
};

const Lib = {
  mode: "folder",
  listing: null,
  rows: [],
  sel: -1,
  sort: { key: null, asc: true },

  init() {
    $("#lib-up").addEventListener("click", () => {
      if (this.mode === "folder" && this.listing && this.listing.parent) this.open(this.listing.parent);
    });
    $("#lib-history").addEventListener("click", () => this.showHistory());
    $("#lib-search").addEventListener("input", () => { this.sel = -1; this.render(); });
    $$(".lib-table th[data-sort]").forEach((th) => th.addEventListener("click", () => {
      const key = th.dataset.sort;
      if (this.sort.key === key) {
        if (!this.sort.asc) this.sort = { key: null, asc: true };
        else this.sort.asc = false;
      } else this.sort = { key, asc: true };
      this.render();
    }));

    const body = $("#lib-body");
    body.addEventListener("click", (e) => {
      const tr = e.target.closest("tr");
      if (!tr) return;
      const idx = +tr.dataset.idx;
      const lb = e.target.closest(".load-btn");
      if (lb) {
        S.shift = e.shiftKey;
        loadPath(+lb.dataset.deck, this.rows[idx].path);
        return;
      }
      this.select(idx);
    });
    body.addEventListener("dblclick", (e) => {
      const tr = e.target.closest("tr");
      if (!tr || e.target.closest(".load-btn")) return;
      S.shift = e.shiftKey;
      this.activate(+tr.dataset.idx);
    });
    body.addEventListener("dragstart", (e) => {
      const tr = e.target.closest("tr");
      if (!tr) return;
      const row = this.rows[+tr.dataset.idx];
      if (!row || row.dir) { e.preventDefault(); return; }
      e.dataTransfer.setData("application/x-bubba-path", row.path);
      e.dataTransfer.setData("text/plain", row.title || row.name);
      e.dataTransfer.effectAllowed = "copy";
    });

    api("/api/roots").then((data) => {
      this.renderRoots(data.roots || []);
      const start = data.last_folder || (data.roots[0] && data.roots[0].path);
      if (start) this.open(start).catch(() => data.roots[0] && this.open(data.roots[0].path));
      else this.showEmpty("No se encontraron carpetas.");
    }).catch((e) => this.showEmpty(`No se pudo leer la biblioteca: ${e.message}`));
  },

  renderRoots(roots) {
    const nav = $("#lib-roots");
    nav.innerHTML = "";
    roots.forEach((root) => {
      const b = document.createElement("button");
      b.className = "lib-root";
      b.dataset.path = root.path;
      b.innerHTML = `<span class="ico-sm">${ICON[root.icon] || ICON.folder}</span><span></span>`;
      b.lastChild.textContent = root.name;
      b.title = root.path;
      b.addEventListener("click", () => this.open(root.path));
      nav.appendChild(b);
    });
  },

  open(path) {
    return api("/api/browse", { path }).then((data) => {
      this.mode = "folder";
      this.listing = data;
      this.sel = -1;
      $("#lib-search").value = "";
      this.renderCrumbs();
      this.render();
      $("#lib-table-wrap").scrollTop = 0;
    }).catch((e) => {
      toast(e.message, "error");
      throw e;
    });
  },

  showHistory() {
    this.mode = "history";
    this.sel = -1;
    $("#lib-search").value = "";
    $("#lib-crumbs").innerHTML = `<span class="crumb">Historial de pistas cargadas</span>`;
    this.render();
  },

  renderCrumbs() {
    const box = $("#lib-crumbs");
    box.innerHTML = "";
    const path = this.listing.path;
    const parts = path.split("\\").filter(Boolean);
    let acc = "";
    parts.forEach((p, k) => {
      acc = k === 0 ? (p.endsWith(":") ? p + "\\" : p) : (acc.endsWith("\\") ? acc + p : acc + "\\" + p);
      const target = acc;
      if (k > 0) box.insertAdjacentHTML("beforeend", `<span class="crumb-sep">›</span>`);
      const b = document.createElement("button");
      b.className = "crumb";
      b.textContent = p;
      b.addEventListener("click", () => this.open(target));
      box.appendChild(b);
    });
    // Highlight only the most specific source containing this folder
    const lower = path.toLowerCase();
    let best = null;
    $$("#lib-roots .lib-root").forEach((b) => {
      const root = b.dataset.path.toLowerCase();
      if (lower.startsWith(root) && (!best || root.length > best.dataset.path.length)) best = b;
    });
    $$(".lib-root").forEach((b) => b.classList.toggle("on", b === best));
    requestAnimationFrame(() => { box.scrollLeft = box.scrollWidth; });
  },

  filteredRows() {
    const q = $("#lib-search").value.trim().toLowerCase();
    let files, dirs = [];
    if (this.mode === "history") files = History.items.slice();
    else {
      if (!this.listing) return [];
      files = this.listing.files.slice();
      dirs = this.listing.dirs.map((d) => ({ ...d, dir: true }));
    }
    if (q) {
      const match = (r) => `${r.title || ""} ${r.artist || ""} ${r.name || ""} ${r.camelot || ""}`.toLowerCase().includes(q);
      files = files.filter(match);
      dirs = dirs.filter((d) => d.name.toLowerCase().includes(q));
    }
    if (this.sort.key) {
      const k = this.sort.key, dir = this.sort.asc ? 1 : -1;
      files.sort((a, b) => {
        const va = a[k], vb = b[k];
        if (va === undefined || va === null) return 1;
        if (vb === undefined || vb === null) return -1;
        return (typeof va === "number" ? va - vb : String(va).localeCompare(String(vb), "es", { numeric: true })) * dir;
      });
    }
    return dirs.concat(files);
  },

  render() {
    $("#lib-history").classList.toggle("on", this.mode === "history");
    if (this.mode === "history") $$("#lib-roots .lib-root").forEach((b) => b.classList.remove("on"));
    $$(".lib-table th[data-sort]").forEach((th) => {
      th.classList.toggle("sorted", this.sort.key === th.dataset.sort);
      th.classList.toggle("asc", this.sort.key === th.dataset.sort && this.sort.asc);
    });

    this.rows = this.filteredRows();
    const ref = this.referenceBpm();
    const body = $("#lib-body");
    const frag = document.createDocumentFragment();
    this.rows.forEach((row, idx) => {
      const tr = document.createElement("tr");
      tr.dataset.idx = idx;
      if (row.dir) {
        tr.className = "dir";
        tr.innerHTML = `<td class="c-load"><span class="folder-ico">▸</span></td><td class="c-title" colspan="6"></td>`;
        tr.children[1].textContent = row.name;
      } else {
        tr.draggable = true;
        const color = camelotColor(row.camelot);
        const bpmMatch = ref && row.bpm && Math.abs(row.bpm / ref - 1) < 0.04;
        tr.innerHTML = `
          <td class="c-load"><div class="load-btns"><button class="load-btn a" data-deck="0" title="Cargar en Deck 1">1</button><button class="load-btn b" data-deck="1" title="Cargar en Deck 2">2</button></div></td>
          <td class="c-title"></td>
          <td class="c-artist"></td>
          <td class="c-bpm${bpmMatch ? " bpm-match" : ""}">${row.bpm ? row.bpm.toFixed(1) : ""}</td>
          <td class="c-key">${color ? `<span class="key-tag" style="--key-color:${color}">${row.camelot}</span>` : ""}</td>
          <td class="c-time">${row.duration ? fmtTime(row.duration, false) : ""}</td>
          <td class="c-ext">${row.ext || ""}</td>`;
        tr.children[1].textContent = row.title || row.name;
        tr.children[1].title = row.path;
        tr.children[2].textContent = row.artist || "";
      }
      if (idx === this.sel) tr.classList.add("sel");
      frag.appendChild(tr);
    });
    body.replaceChildren(frag);
    this.refreshLoadedMarks();

    if (!this.rows.length) {
      this.showEmpty(this.mode === "history"
        ? "Todavía no has cargado pistas en esta sesión."
        : ($("#lib-search").value ? "Sin resultados en esta carpeta." : "Esta carpeta no tiene audio.<br>Formatos: MP3, WAV, FLAC, AIFF, M4A/AAC, ALAC, OGG."));
    } else $("#lib-empty").classList.add("hidden");
  },

  showEmpty(html) {
    const el = $("#lib-empty");
    el.innerHTML = html;
    el.classList.remove("hidden");
  },

  /** BPM of the deck currently playing, to highlight tracks that mix well */
  referenceBpm() {
    const m = S.mixer;
    if (!m) return null;
    const d = m.deck1.is_playing ? m.deck1 : m.deck2.is_playing ? m.deck2 : null;
    return d && d.loaded ? d.effective_bpm : null;
  },

  refreshLoadedMarks() {
    $$("#lib-body tr").forEach((tr) => {
      const row = this.rows[+tr.dataset.idx];
      if (!row || row.dir) return;
      tr.classList.toggle("loaded-0", !!(S.tracks[0] && S.tracks[0].path === row.path));
      tr.classList.toggle("loaded-1", !!(S.tracks[1] && S.tracks[1].path === row.path));
    });
  },

  select(idx) {
    this.sel = clamp(idx, 0, this.rows.length - 1);
    $$("#lib-body tr").forEach((tr) => tr.classList.toggle("sel", +tr.dataset.idx === this.sel));
    const tr = $(`#lib-body tr[data-idx="${this.sel}"]`);
    if (tr) tr.scrollIntoView({ block: "nearest" });
  },

  moveSel(delta) {
    if (!this.rows.length) return;
    this.select(this.sel < 0 ? (delta > 0 ? 0 : this.rows.length - 1) : this.sel + delta);
  },

  /** Double click / Enter: open folder, or load into the deck that is not playing */
  activate(idx) {
    const row = this.rows[idx];
    if (!row) return;
    if (row.dir) { this.open(row.path); return; }
    const m = S.mixer;
    let deck = 0;
    if (m) {
      if (m.deck1.is_playing && !m.deck2.is_playing) deck = 1;
      else if (!m.deck1.loaded) deck = 0;
      else if (!m.deck2.loaded) deck = 1;
    }
    loadPath(deck, row.path);
  },

  loadSelected(deck) {
    const row = this.rows[this.sel];
    if (!row) { toast("Selecciona una pista en la biblioteca.", "warn"); return; }
    if (row.dir) { this.open(row.path); return; }
    loadPath(deck, row.path);
  },
};

// ======================================================================
// Status, settings, help
// ======================================================================

function onRouting(msg) {
  S.routing = msg;
  const pill = $("#audio-pill");
  const name = (msg.master_device || "Sin salida").replace(/\s*\(.*?\)\s*/g, " ").trim();
  const shortName = name.length > 22 ? name.slice(0, 21) + "…" : name;
  setText($("#audio-text"), msg.master_device ? `${shortName} · ${(msg.sample_rate / 1000).toFixed(1)} kHz` : "Sin audio");
  pill.classList.toggle("err", !!msg.error || !msg.master_device);
  pill.title = `Master: ${msg.master_device || "—"}\nAudífonos: ${msg.headphone_device || (msg.four_channel_cue ? "canales 3-4 del master" : "—")}`;
  $("#version-label").textContent = msg.version ? `Bubba DJ Pro v${msg.version}` : "";
  if (msg.error) toast(`Audio: ${msg.error}`, "error", 7000);
  if (!$("#settings-modal").classList.contains("hidden")) fillSettings();
}

function onMidi(msg) {
  S.midi = msg;
  const pill = $("#midi-pill");
  pill.classList.toggle("ok", !!msg.connected);
  pill.classList.toggle("warn", !msg.connected);
  setText($("#midi-text"), msg.connected ? (msg.name || "Controlador") : "Sin controlador");
  pill.title = msg.connected ? `Conectado: ${msg.name}` : "Conecta tu Pioneer DDJ-FLX4 por USB; se detecta automáticamente.";
}

function fillSettings() {
  const r = S.routing || { devices: [] };
  const selM = $("#sel-master"), selH = $("#sel-headphone");
  selM.innerHTML = "";
  selH.innerHTML = "";
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "Ninguna (pre-escucha por canales 3-4 del master, si existen)";
  selH.appendChild(none);
  (r.devices || []).forEach((d) => {
    const a = document.createElement("option");
    a.value = d; a.textContent = d; a.selected = d === r.master_device;
    selM.appendChild(a);
    const b = document.createElement("option");
    b.value = d; b.textContent = d; b.selected = d === r.headphone_device;
    selH.appendChild(b);
  });
  const info = $("#routing-info");
  info.innerHTML = r.master_device
    ? `<b>Master:</b> ${escapeHtml(r.master_device)} · ${r.sample_rate} Hz · ${r.master_channels} canales<br>
       <b>Audífonos:</b> ${r.headphone_device ? escapeHtml(r.headphone_device) : (r.four_channel_cue ? "canales 3-4 del dispositivo master (jack de audífonos de la controladora)" : "sin salida independiente")}`
    : "No hay ninguna salida de audio activa.";
  const err = $("#routing-error");
  err.classList.toggle("hidden", !r.error);
  err.textContent = r.error || "";
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function openModal(id) {
  if (id === "settings-modal") fillSettings();
  $("#" + id).classList.remove("hidden");
}

function closeModals() {
  $$(".modal").forEach((m) => m.classList.add("hidden"));
}

function initChrome() {
  $("#audio-pill").addEventListener("click", () => openModal("settings-modal"));
  $("#btn-settings").addEventListener("click", () => openModal("settings-modal"));
  $("#btn-help").addEventListener("click", () => openModal("help-modal"));
  $$("#sync-mode button").forEach((b) => b.addEventListener("click", () => send({ action: "SetSyncMode", mode: b.dataset.mode })));
  $("#midi-pill").addEventListener("click", () => {
    toast(S.midi && S.midi.connected ? `${S.midi.name} conectada y mapeada.` : "No hay controlador. Conecta la DDJ-FLX4 por USB: se detecta sola en unos segundos.", "info");
  });
  $("#btn-fullscreen").addEventListener("click", () => {
    if (document.fullscreenElement) document.exitFullscreen();
    else document.documentElement.requestFullscreen().catch(() => {});
  });
  $$(".modal").forEach((m) => m.addEventListener("pointerdown", (e) => { if (e.target === m) closeModals(); }));
  $$("[data-close]").forEach((b) => b.addEventListener("click", closeModals));
  $("#btn-apply-audio").addEventListener("click", () => {
    send({ action: "SetAudioRouting", master: $("#sel-master").value, headphone: $("#sel-headphone").value });
    toast("Aplicando configuración de audio…", "info", 2000);
    closeModals();
  });
  $("#zoom-in").addEventListener("click", () => setZoom(S.zoomIdx + 1));
  $("#zoom-out").addEventListener("click", () => setZoom(S.zoomIdx - 1));

  // Library resizer
  const app = $("#app");
  const savedH = store.get("libHeight", null);
  if (savedH) app.style.setProperty("--lib-h", `${savedH}px`);
  const rz = $("#lib-resizer");
  rz.addEventListener("pointerdown", (e) => {
    rz.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startH = $("#library").getBoundingClientRect().height;
    const move = (ev) => {
      const maxH = window.innerHeight - 46 - $(".console").getBoundingClientRect().height - 120;
      const hgt = clamp(startH + (startY - ev.clientY), 110, Math.max(110, maxH));
      app.style.setProperty("--lib-h", `${hgt}px`);
      store.set("libHeight", Math.round(hgt));
    };
    const up = () => {
      rz.removeEventListener("pointermove", move);
      rz.removeEventListener("pointerup", up);
    };
    rz.addEventListener("pointermove", move);
    rz.addEventListener("pointerup", up);
  });

  // Files dropped anywhere else: load into the deck under that half of the screen
  window.addEventListener("dragover", (e) => e.preventDefault());
  window.addEventListener("drop", (e) => {
    e.preventDefault();
    if (!e.dataTransfer.files.length) return;
    S.shift = e.shiftKey;
    uploadFile(e.clientX > window.innerWidth / 2 ? 1 : 0, e.dataTransfer.files[0]);
  });
}

// ======================================================================
// Keyboard
// ======================================================================

const KEYMAP = {
  KeyQ: [0, "cue"], KeyW: [0, "play"], KeyE: [0, "sync"], KeyA: [0, "half"], KeyS: [0, "double"], KeyD: [0, "loop"],
  Digit1: [0, "hc", 0], Digit2: [0, "hc", 1], Digit3: [0, "hc", 2], Digit4: [0, "hc", 3],
  KeyU: [1, "cue"], KeyI: [1, "play"], KeyO: [1, "sync"], KeyJ: [1, "half"], KeyK: [1, "double"], KeyL: [1, "loop"],
  Digit7: [1, "hc", 0], Digit8: [1, "hc", 1], Digit9: [1, "hc", 2], Digit0: [1, "hc", 3],
};

function initKeyboard() {
  window.addEventListener("keydown", (e) => {
    S.shift = e.shiftKey;
    const typing = e.target.matches("input, select, textarea");
    if (e.key === "Escape") {
      closeModals();
      if (typing) e.target.blur();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.code === "KeyF") {
      e.preventDefault();
      $("#lib-search").focus();
      $("#lib-search").select();
      return;
    }
    if (typing && e.code !== "ArrowUp" && e.code !== "ArrowDown" && e.code !== "Enter") return;
    if (e.key === "?") { openModal("help-modal"); return; }
    if (e.code === "F11") return;

    if (e.code === "ArrowDown" || e.code === "ArrowUp") {
      e.preventDefault();
      Lib.moveSel(e.code === "ArrowDown" ? 1 : -1);
      return;
    }
    if (e.code === "Enter") {
      if (Lib.sel >= 0) { e.preventDefault(); Lib.activate(Lib.sel); }
      return;
    }
    if (e.shiftKey && (e.code === "ArrowLeft" || e.code === "ArrowRight")) {
      e.preventDefault();
      Lib.loadSelected(e.code === "ArrowLeft" ? 0 : 1);
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;

    const map = KEYMAP[e.code];
    if (!map || e.repeat) return;
    e.preventDefault();
    const [deck, act, arg] = map;
    const d = deckState(deck);
    switch (act) {
      case "cue": send({ action: "Cue", deck, pressed: true }); break;
      case "play": send({ action: "PlayPause", deck }); break;
      case "sync": send({ action: "Sync", deck }); break;
      case "half": send({ action: "LoopResize", deck, factor: 0.5 }); break;
      case "double": send({ action: "LoopResize", deck, factor: 2 }); break;
      case "loop": send({ action: "SetLoop", deck, beats: d ? d.loop_size_beats : 4 }); break;
      case "hc":
        if (e.shiftKey) send({ action: "DeleteHotCue", deck, pad_index: arg });
        else send({ action: "HotCue", deck, pad_index: arg, pressed: true });
        break;
    }
  });
  window.addEventListener("keyup", (e) => {
    S.shift = e.shiftKey;
    const map = KEYMAP[e.code];
    if (map && map[1] === "cue") send({ action: "Cue", deck: map[0], pressed: false });
  });
  window.addEventListener("blur", () => { S.shift = false; });
  document.addEventListener("contextmenu", (e) => {
    if (!e.target.matches("input")) e.preventDefault();
  });
}

// ======================================================================
// Boot
// ======================================================================

function boot() {
  buildMixer();
  buildDeck(0);
  buildDeck(1);
  initChrome();
  initKeyboard();
  Lib.init();
  updateKeyCompat();
  $("#offline").classList.remove("hidden");
  connect();
  requestAnimationFrame(frame);
}

boot();
