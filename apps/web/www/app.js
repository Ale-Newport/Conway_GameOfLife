// Front end of the Emergence Lab web demo. The simulation itself is the Rust
// `cellular-automata` crate compiled to WebAssembly (apps/web/src/lib.rs);
// this file only draws cells and wires up the controls.
import init, { WebLife, patternList, presetList } from "./pkg/emergence_web.js";

const WIDTH = 256;
const HEIGHT = 160;
const SPEEDS = [1, 2, 4, 8, 16, 32, 64, 128];
const START_PATTERN = "gosper-glider-gun";

const $ = (id) => document.getElementById(id);
const canvas = $("world");
const ctx = canvas.getContext("2d");

function showError(message) {
  const el = $("error");
  el.textContent = message;
  el.hidden = false;
}

function hexToRgb(hex) {
  const n = parseInt(hex.trim().replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

async function main() {
  await init();

  const life = new WebLife(WIDTH, HEIGHT, "conway", 0, BigInt(Date.now() % 1_000_000));
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  const image = ctx.createImageData(WIDTH, HEIGHT);
  // Brightness of recently dead cells, purely cosmetic ("trails").
  const glow = new Float32Array(WIDTH * HEIGHT);

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let running = !reduceMotion;
  let speed = 1;
  let seed = 1;
  let rateGens = 0;
  let rateStart = performance.now();

  // ---- controls ---------------------------------------------------------
  const patterns = patternList().map((line) => {
    const [id, name, category, description] = line.split("|");
    return { id, name, category, description };
  });
  const patternSelect = $("pattern");
  const byCategory = new Map();
  for (const p of patterns) {
    if (!byCategory.has(p.category)) byCategory.set(p.category, []);
    byCategory.get(p.category).push(p);
  }
  for (const [category, items] of byCategory) {
    const group = document.createElement("optgroup");
    group.label = category.replace("-", " ");
    for (const p of items) group.append(new Option(p.name, p.id));
    patternSelect.append(group);
  }

  const presetSelect = $("preset");
  for (const line of presetList()) {
    const [id, label] = line.split("|");
    presetSelect.append(new Option(label, id));
  }

  function describePattern() {
    const p = patterns.find((x) => x.id === patternSelect.value);
    $("pattern-desc").textContent = p ? p.description : "";
  }

  function loadPattern(id) {
    try {
      life.loadPattern(id);
      glow.fill(0);
      draw();
    } catch (e) {
      showError(String(e.message ?? e));
    }
  }

  function setRunning(value) {
    running = value;
    $("play").textContent = running ? "Pause" : "Play";
    $("play").setAttribute("aria-pressed", String(running));
    rateGens = 0;
    rateStart = performance.now();
  }

  function stepOnce() {
    life.step(1);
    draw();
  }

  function randomize() {
    life.randomize(0.3, BigInt(Date.now() % 1_000_000 + seed++));
    glow.fill(0);
    draw();
  }

  $("play").addEventListener("click", () => setRunning(!running));
  $("step").addEventListener("click", () => { setRunning(false); stepOnce(); });
  $("random").addEventListener("click", randomize);
  $("clear").addEventListener("click", () => { life.clear(); glow.fill(0); draw(); });
  $("speed").addEventListener("input", (e) => {
    speed = SPEEDS[Number(e.target.value)];
    $("speed-out").textContent = String(speed);
  });
  patternSelect.addEventListener("change", () => { describePattern(); loadPattern(patternSelect.value); });
  presetSelect.addEventListener("change", () => {
    try {
      life.setPreset(presetSelect.value);
    } catch (e) {
      showError(String(e.message ?? e));
    }
  });

  window.addEventListener("keydown", (e) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.target instanceof HTMLSelectElement || e.target instanceof HTMLInputElement) return;
    const key = e.key.toLowerCase();
    if (key === " ") { e.preventDefault(); setRunning(!running); }
    else if (key === "n") { setRunning(false); stepOnce(); }
    else if (key === "r") randomize();
    else if (key === "c") { life.clear(); glow.fill(0); draw(); }
  });

  // ---- drawing on the canvas -------------------------------------------
  let painting = null; // null, or true (draw) / false (erase)
  let last = null;
  function cellAt(event) {
    const rect = canvas.getBoundingClientRect();
    return [
      Math.floor(((event.clientX - rect.left) / rect.width) * WIDTH),
      Math.floor(((event.clientY - rect.top) / rect.height) * HEIGHT),
    ];
  }
  function paintTo([x, y]) {
    // Bresenham from the previous point so fast strokes stay connected.
    let [x0, y0] = last ?? [x, y];
    const dx = Math.abs(x - x0), dy = -Math.abs(y - y0);
    const sx = x0 < x ? 1 : -1, sy = y0 < y ? 1 : -1;
    let err = dx + dy;
    for (;;) {
      life.setCell(x0, y0, painting);
      if (x0 === x && y0 === y) break;
      const e2 = 2 * err;
      if (e2 >= dy) { err += dy; x0 += sx; }
      if (e2 <= dx) { err += dx; y0 += sy; }
    }
    last = [x, y];
    draw();
  }
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  canvas.addEventListener("pointerdown", (e) => {
    canvas.setPointerCapture(e.pointerId);
    painting = !(e.button === 2 || e.shiftKey);
    last = null;
    paintTo(cellAt(e));
  });
  canvas.addEventListener("pointermove", (e) => { if (painting !== null) paintTo(cellAt(e)); });
  const stop = () => { painting = null; last = null; };
  canvas.addEventListener("pointerup", stop);
  canvas.addEventListener("pointercancel", stop);

  // ---- rendering --------------------------------------------------------
  const styles = getComputedStyle(document.documentElement);
  const [ar, ag, ab] = hexToRgb(styles.getPropertyValue("--accent") || "#48e5c2");
  const bg = [5, 8, 11];

  function draw() {
    const cells = life.cells();
    const data = image.data;
    for (let i = 0; i < cells.length; i++) {
      let r, g, b;
      if (cells[i]) {
        glow[i] = 1;
        r = 235; g = 255; b = 250;
      } else {
        const t = glow[i] * 0.55;
        glow[i] *= 0.82;
        r = bg[0] + (ar - bg[0]) * t;
        g = bg[1] + (ag - bg[1]) * t;
        b = bg[2] + (ab - bg[2]) * t;
      }
      const o = i * 4;
      data[o] = r; data[o + 1] = g; data[o + 2] = b; data[o + 3] = 255;
    }
    ctx.putImageData(image, 0, 0);
    $("gen").textContent = life.generation().toLocaleString();
    $("pop").textContent = life.population().toLocaleString();
    $("engine").textContent = life.engine();
  }

  function frame(now) {
    if (running) {
      rateGens += life.step(speed);
      draw();
      const elapsed = now - rateStart;
      if (elapsed > 500) {
        $("rate").textContent = Math.round((rateGens * 1000) / elapsed).toLocaleString();
        rateGens = 0;
        rateStart = now;
      }
    }
    requestAnimationFrame(frame);
  }

  patternSelect.value = START_PATTERN;
  describePattern();
  loadPattern(START_PATTERN);
  setRunning(running);
  requestAnimationFrame(frame);
}

main().catch((e) => showError(`Could not start the WebAssembly module: ${e.message ?? e}`));
