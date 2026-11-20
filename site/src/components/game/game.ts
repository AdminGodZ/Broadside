// The browser game's logic and drawing. The engine is the Rust crate
// broadside-wasm, built to broadside.wasm by scripts/prepare.sh: a plain C ABI,
// numbers in and out (see crates/wasm/src/lib.rs). Cells are row * 10 + col;
// placement slots are top-left cell * 2 + vertical; outcomes 0 miss, 1 hit,
// 2 + ship for a sinking.
import wasmUrl from "./broadside.wasm?url";

interface Engine {
  bs_new_game(seed: number): void;
  bs_random_fleet(): void;
  bs_offer_slot(ship: number): number;
  bs_set_fleet(a: number, b: number, c: number, d: number, e: number): number;
  bs_player_fire(cell: number): number;
  bs_player_won(): number;
  bs_engine_fire(): number;
  bs_engine_won(): number;
  bs_engine_slot(ship: number): number;
  bs_finish(): void;
  bs_update_heat(): void;
  bs_heat(cell: number): number;
}

type Phase = "loading" | "placing" | "battle" | "over";
interface Side { shots: Map<number, number>; sunk: number[]; count: number; last: number | null; fresh: number | null }
interface Chart { cells: HTMLButtonElement[]; layer: HTMLDivElement }

const SHIPS: [string, number][] = [["Carrier", 5], ["Battleship", 4], ["Cruiser", 3], ["Submarine", 3], ["Destroyer", 2]];
const ROWS = "ABCDEFGHIJ";
const name = (c: number) => ROWS[Math.floor(c / 10)] + (c % 10 + 1);
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const KEYS = matchMedia("(hover: hover)").matches; // keyboard hints only where there is likely a keyboard
const OUT = (o: number) => (o === 0 ? "miss" : o === 1 ? "hit" : "sunk");

/** Ship cells of a placement slot, or null off the board. */
function cellsOf(len: number, slot: number): number[] | null {
  const start = slot >> 1, vertical = slot & 1, r = Math.floor(start / 10), c = start % 10;
  if (vertical ? r + len > 10 : c + len > 10) return null;
  return Array.from({ length: len }, (_, k) => start + (vertical ? 10 * k : k));
}
const placed = (len: number, slot: number) => cellsOf(len, slot) as number[]; // slots in the fleet are valid
const newSide = (): Side => ({ shots: new Map(), sunk: [], count: 0, last: null, fresh: null });

let E: Engine;
const S = {
  phase: "loading" as Phase,
  fleet: [null, null, null, null, null] as (number | null)[],
  selected: null as number | null, // the ship in hand while placing
  vertical: false,
  hover: null as number | null,
  fresh: null as number | "all" | null,
  reveal: null as number[] | null,
  you: newSide(), // your shots at the engine
  it: newSide(), // the engine's at you
  yourTurn: true,
  thinking: false,
  record: { w: 0, l: 0, winShots: [] as number[] },
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}) {
  return Object.assign(document.createElement(tag), props);
}

function buildChart(root: HTMLElement, onCell: (i: number | null, kind: "click" | "enter" | "leave") => void): Chart {
  root.replaceChildren(el("span", { className: "axis" }));
  for (let c = 0; c < 10; c++) root.append(el("span", { className: "axis", textContent: String(c + 1) }));
  for (let r = 0; r < 10; r++) {
    const a = el("span", { className: "axis", textContent: ROWS[r] });
    a.style.gridColumn = "1";
    a.style.gridRow = String(r + 2);
    root.append(a);
  }
  const sea = el("div", { className: "sea" });
  const layer = el("div", { className: "layer" });
  sea.append(layer);
  const cells = Array.from({ length: 100 }, (_, i) => {
    const b = el("button", { className: "cell", type: "button" });
    b.setAttribute("aria-label", name(i));
    b.addEventListener("click", () => onCell(i, "click"));
    b.addEventListener("pointerenter", () => onCell(i, "enter"));
    b.addEventListener("focus", () => onCell(i, "enter"));
    sea.append(b);
    return b;
  });
  sea.addEventListener("pointerleave", () => onCell(null, "leave"));
  root.append(sea);
  return { cells, layer };
}

let target: Chart, home: Chart;

// ---------- placement ----------
function occupiedBy(except: number) {
  const occ = new Map<number, number>();
  S.fleet.forEach((slot, s) => {
    if (slot !== null && s !== except) placed(SHIPS[s][1], slot).forEach((c) => occ.set(c, s));
  });
  return occ;
}
const nextUnplaced = () => {
  const n = S.fleet.findIndex((x) => x === null);
  return n >= 0 ? n : null;
};
function pick(s: number) {
  // Picking the ship already in hand puts it back where it was; the next unplaced ship, if any, comes to hand.
  S.selected = S.selected === s && S.fleet[s] !== null ? nextUnplaced() : s;
  render();
}
function placeAt(cell: number) {
  const s = S.selected;
  if (s === null) {
    // Nothing in hand: a click on a ship picks it up.
    const occ = occupiedBy(-1);
    if (occ.has(cell)) pick(occ.get(cell)!);
    return;
  }
  const occ = occupiedBy(s);
  const slot = cell * 2 + (S.vertical ? 1 : 0);
  const cells = cellsOf(SHIPS[s][1], slot);
  if (cells && !cells.some((c) => occ.has(c))) {
    S.fleet[s] = slot;
    S.fresh = s;
    S.selected = nextUnplaced();
    note(`${SHIPS[s][0]} placed at ${span(cells)}.`);
    render();
  } else if (occ.has(cell)) {
    pick(occ.get(cell)!); // a click on another ship swaps to it; the one in hand stays where it was
  } else {
    flash(cells ? "It would overlap another ship." : `The ${SHIPS[s][0]} runs off the chart there.`);
  }
}
function randomFleet() {
  E.bs_random_fleet();
  S.fleet = SHIPS.map((_, s) => E.bs_offer_slot(s));
  S.selected = null;
  S.fresh = "all";
  note("A random fleet. Move any ship, or start the battle.");
  render();
}
function clearFleet() {
  S.fleet = [null, null, null, null, null];
  S.selected = 0;
  note(null);
  render();
}
function setVertical(v: boolean) {
  S.vertical = v;
  render();
}

// ---------- battle ----------
function startBattle() {
  const [a, b, c, d, e] = S.fleet as number[];
  if (!E.bs_set_fleet(a, b, c, d, e)) return flash("Every ship needs a place on the chart.");
  try {
    localStorage.setItem("broadside.fleet", JSON.stringify(S.fleet));
  } catch {}
  S.phase = "battle";
  S.selected = null;
  S.hover = null;
  S.yourTurn = Math.random() < 0.5;
  log(S.yourTurn ? "Coin toss: you fire first" : "Coin toss: the engine fires first", "");
  render();
  if (!S.yourTurn) engineMove();
}
function playerFire(cell: number) {
  if (S.phase !== "battle" || !S.yourTurn || S.thinking || S.you.shots.has(cell)) return;
  const o = E.bs_player_fire(cell);
  if (o < 0) return;
  S.you.shots.set(cell, o);
  S.you.count++;
  S.you.fresh = cell;
  if (o >= 2) S.you.sunk.push(o - 2);
  log(`${name(cell)} ${o >= 2 ? SHIPS[o - 2][0] + " sunk" : OUT(o)}`, "you " + (o === 1 ? "h" : o >= 2 ? "s" : ""));
  if (E.bs_player_won()) return finish(true);
  S.yourTurn = false;
  render();
  engineMove();
}
function engineMove() {
  S.thinking = true;
  render();
  // Let the page paint "aiming" before the engine takes the main thread.
  setTimeout(() =>
    requestAnimationFrame(() =>
      setTimeout(() => {
        const r = E.bs_engine_fire(), cell = r >> 4, o = r & 15;
        S.it.shots.set(cell, o);
        S.it.count++;
        S.it.last = cell;
        if (o >= 2) S.it.sunk.push(o - 2);
        log(`${name(cell)} ${o >= 2 ? SHIPS[o - 2][0] + " sunk" : OUT(o)}`, "it " + (o === 1 ? "h" : o >= 2 ? "s" : ""));
        S.thinking = false;
        if (E.bs_engine_won()) return finish(false);
        S.yourTurn = true;
        render();
      }, 0),
    ), 220);
}
function finish(youWon: boolean) {
  S.phase = "over";
  S.thinking = false;
  S.reveal = SHIPS.map((_, s) => E.bs_engine_slot(s));
  E.bs_finish();
  if (youWon) {
    S.record.w++;
    S.record.winShots.push(S.you.count);
  } else S.record.l++;
  const hits = (side: Side) => [...side.shots.values()].filter((o) => o > 0).length;
  const b = $("banner");
  b.className = b.className.replace(/\b(win|lose)\b/g, "").trim() + (youWon ? " win" : " lose");
  const left = 17 - hits(S.you);
  b.innerHTML = youWon
    ? `<h3>Victory in ${S.you.count} shots</h3><p>You sank the engine's fleet first. It had ${17 - hits(S.it)} of your ship cells still to find.</p>`
    : `<h3>The engine wins in ${S.it.count} shots</h3><p>You had ${left} ship cell${left === 1 ? "" : "s"} left to find. Its fleet is outlined in enemy waters.</p>`;
  b.hidden = false;
  render();
}
function newGame() {
  E.bs_new_game(Math.random() * 2 ** 52);
  Object.assign(S, { phase: "placing", selected: null, hover: null, reveal: null, thinking: false, fresh: null, you: newSide(), it: newSide() });
  $("log").replaceChildren();
  $("banner").hidden = true;
  let saved: unknown = null;
  try {
    saved = JSON.parse(localStorage.getItem("broadside.fleet") ?? "null");
  } catch {}
  if (Array.isArray(saved) && saved.length === 5 && saved.every((x) => Number.isInteger(x))) {
    S.fleet = saved;
    note("Your last fleet is in position. Move any ship, or start the battle.");
  } else randomFleet();
  render();
}

// ---------- view ----------
let flashTimer: ReturnType<typeof setTimeout> | undefined;
let notice: string | null = null;
const note = (text: string | null) => (notice = text);
const sub = (text: string) => el("span", { className: "msg-sub", textContent: text });
function flash(text: string) {
  $("msg").replaceChildren(text, sub(KEYS ? "Try another cell, or turn it with R." : "Try another cell, or turn it."));
  clearTimeout(flashTimer);
  flashTimer = setTimeout(render, 1800);
}
function log(text: string, cls: string) {
  $("log").prepend(el("li", { textContent: text, className: cls }));
}
function hull(layer: HTMLElement, len: number, slot: number, cls: string) {
  const start = slot >> 1, v = slot & 1, r = Math.floor(start / 10), c = start % 10;
  const d = el("div", { className: "hull " + cls });
  d.style.cssText = `left:${c * 10}%;top:${r * 10}%;width:${(v ? 1 : len) * 10}%;height:${(v ? len : 1) * 10}%`;
  layer.append(d);
}
const span = (cells: number[]) => `${name(cells[0])}–${name(cells[cells.length - 1])}`;
const pips = (len: number) => {
  const p = el("span", { className: "pips" });
  for (let k = 0; k < len; k++) p.append(el("i"));
  return p;
};

function renderDock() {
  $("dock-ships").replaceChildren(
    ...SHIPS.map(([n, len], s) => {
      const slot = S.fleet[s], sel = S.selected === s;
      const b = el("button", { type: "button", className: "ship-row" + (slot !== null ? " placed" : "") + (sel ? " sel" : "") });
      b.setAttribute("aria-pressed", String(sel));
      const shape = el("span", { className: "shape" });
      for (let k = 0; k < len; k++) shape.append(el("i"));
      const label = el("span", { className: "name", textContent: n });
      label.append(el("small", { textContent: `${len} cells` }));
      const where = sel ? (slot !== null ? "Moving" : "In hand") : slot !== null ? span(placed(len, slot)) : "Not placed";
      b.append(shape, label, el("span", { className: "where", textContent: where }));
      b.addEventListener("click", () => pick(s));
      const li = el("li");
      li.append(b);
      return li;
    }),
  );
  const ready = S.fleet.every((x) => x !== null);
  $<HTMLButtonElement>("start").disabled = !ready || S.selected !== null;
  $("dir-h").setAttribute("aria-pressed", String(!S.vertical));
  $("dir-v").setAttribute("aria-pressed", String(S.vertical));
  const s = S.selected;
  $("dock-hint").textContent =
    s !== null
      ? `Click a cell for the ${SHIPS[s][0]}'s ${S.vertical ? "top" : "left"} end.` +
        (S.fleet[s] !== null ? (KEYS ? " Esc puts it back." : " Tap it in this list to put it back.") : "")
      : "Click a ship on the chart, or in this list, to move it.";
}

function renderTally(list: HTMLElement, sunk: number[]) {
  list.replaceChildren(
    ...SHIPS.map(([n, len], s) => {
      const li = el("li", { className: "tally-ship" + (sunk.includes(s) ? " down" : "") });
      li.append(pips(len), n);
      return li;
    }),
  );
}

function render() {
  const placing = S.phase === "placing";
  const game = document.querySelector(".game")!;
  game.classList.toggle("placing", placing);
  game.classList.toggle("battle", !placing);
  $("target-col").hidden = placing;
  $("dock").hidden = !placing;
  $("my-fleet").hidden = placing;
  $("heat-toggle").hidden = placing;
  $("log-box").hidden = placing;
  const heatOn = $<HTMLInputElement>("heat").checked;
  $("heat-scale").hidden = !heatOn;

  // Enemy waters
  target.layer.replaceChildren();
  S.reveal?.forEach((slot, s) => hull(target.layer, SHIPS[s][1], slot, S.you.sunk.includes(s) ? "sunk" : "reveal"));
  target.cells.forEach((b, i) => {
    const o = S.you.shots.get(i);
    b.className =
      "cell" +
      (o === undefined ? (S.phase === "battle" && S.yourTurn && !S.thinking ? " open" : "") : o === 0 ? " miss" : " hit") +
      (o !== undefined && o >= 2 && !S.reveal ? " sinker" : "") +
      (S.you.fresh === i ? " fresh" : "");
    b.disabled = S.phase !== "battle";
    b.setAttribute("aria-label", name(i) + (o === undefined ? "" : ", " + (o >= 2 ? SHIPS[o - 2][0] + " sunk" : OUT(o))));
  });
  S.you.fresh = null;

  // Your fleet
  home.layer.replaceChildren();
  const occ = occupiedBy(-1);
  const hoverShip = placing && S.selected === null && S.hover !== null ? occ.get(S.hover) ?? null : null;
  S.fleet.forEach((slot, s) => {
    if (slot === null) return;
    const cls = S.it.sunk.includes(s) ? "sunk" : S.selected === s ? "lifted" : placing ? "movable" + (hoverShip === s ? " hover" : "") : "";
    hull(home.layer, SHIPS[s][1], slot, cls + (S.fresh === s || S.fresh === "all" ? " fresh" : ""));
  });
  S.fresh = null;
  if (placing && S.selected !== null && S.hover !== null) {
    const len = SHIPS[S.selected][1], slot = S.hover * 2 + (S.vertical ? 1 : 0), cells = cellsOf(len, slot);
    const others = occupiedBy(S.selected);
    if (cells) hull(home.layer, len, slot, "ghost" + (cells.some((c) => others.has(c)) ? " bad" : ""));
    else {
      // Off the chart: show what fits, marked as not allowed.
      const r = Math.floor(S.hover / 10), c = S.hover % 10;
      hull(home.layer, S.vertical ? 10 - r : 10 - c, slot, "ghost bad");
    }
  }
  const showHeat = heatOn && !placing;
  if (showHeat) E.bs_update_heat();
  home.cells.forEach((b, i) => {
    const o = S.it.shots.get(i);
    b.className = "cell" + (o === undefined ? "" : o === 0 ? " miss" : " hit") + (S.it.last === i ? " fresh" : "");
    // Not disabled in battle (a disabled cell gets no hover, so no figure); just out of the tab order.
    b.tabIndex = placing ? 0 : -1;
    b.replaceChildren();
    let chance = "";
    if (showHeat && o === undefined) {
      const h = E.bs_heat(i);
      chance = h <= 0 ? "0%" : h >= 1 ? "100%" : h < 0.005 ? "under 1%" : h > 0.995 ? "over 99%" : Math.round(h * 100) + "%";
      const d = el("span", { className: "heat" });
      d.style.opacity = (h * 0.85).toFixed(3);
      b.append(d, el("span", { className: "pct", textContent: `${name(i)}: ${chance}` }));
    }
    if (S.it.last === i && !placing) b.append(el("span", { className: "ring" }));
    const ship = occ.get(i);
    b.setAttribute(
      "aria-label",
      name(i) + (ship !== undefined ? ", " + SHIPS[ship][0] : "") + (o === undefined ? "" : ", " + OUT(o)) + (chance ? `, engine's chance ${chance}` : ""),
    );
  });

  if (placing) renderDock();
  else {
    renderTally($("enemy-fleet"), S.you.sunk);
    renderTally($("my-fleet"), S.it.sunk);
  }
  $("you-shots").textContent = `${S.you.count} shot${S.you.count === 1 ? "" : "s"}`;
  $("it-shots").textContent = placing
    ? `${S.fleet.filter((x) => x !== null).length} of 5 placed`
    : `engine: ${S.it.count} shot${S.it.count === 1 ? "" : "s"}`;

  // Status line and actions
  const msg = $("msg");
  $("resign").hidden = S.phase !== "battle";
  $("new-game").hidden = S.phase !== "over";
  if (placing) {
    const ready = S.fleet.every((x) => x !== null);
    if (S.selected !== null) {
      const [n, len] = SHIPS[S.selected];
      const turn = KEYS ? "Press R to turn it." : "Turn it with Across or Down.";
      msg.replaceChildren(S.fleet[S.selected] === null ? `Place your ${n}` : `Move your ${n}`, sub(notice ?? `${len} cells, ${S.vertical ? "down" : "across"}. ${turn}`));
    } else msg.replaceChildren(ready ? "Your fleet is ready" : "Place your fleet", sub(notice ?? "Move any ship, or start the battle."));
    notice = null;
  } else if (S.phase === "battle") {
    msg.replaceChildren(...(S.thinking ? ["The engine is aiming…", sub("It weighs every fleet that could be hiding.")] : ["Your shot", sub("Pick a cell in enemy waters.")]));
  } else if (S.phase === "over") {
    msg.replaceChildren("Game over", sub("Your fleet stays where it was for the next game."));
  }
  $("rec-w").textContent = String(S.record.w);
  $("rec-l").textContent = String(S.record.l);
  const ws = S.record.winShots;
  $("rec-avg").textContent = ws.length ? (ws.reduce((a, b) => a + b, 0) / ws.length).toFixed(1) : "–";
}

// ---------- boot ----------
async function loadEngine(): Promise<Engine> {
  // The offline single-file version (scripts/offline.mjs) carries the engine inline.
  const inline = (window as { BROADSIDE_WASM?: string }).BROADSIDE_WASM;
  const bytes = inline ? Uint8Array.from(atob(inline), (ch) => ch.charCodeAt(0)) : await (await fetch(wasmUrl)).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return instance.exports as unknown as Engine;
}

export async function start() {
  target = buildChart($("target"), (i, kind) => {
    if (kind === "click" && i !== null) playerFire(i);
  });
  home = buildChart($("home"), (i, kind) => {
    if (S.phase !== "placing") return;
    if (kind === "click" && i !== null) placeAt(i);
    else if (S.hover !== i) {
      S.hover = i;
      render();
    }
  });
  // Keyboard navigation (Tab, arrows) turns focus rings on; a pointer turns them off again.
  document.addEventListener("keydown", (e) => {
    if (e.key === "Tab" || e.key.startsWith("Arrow")) document.documentElement.classList.add("kbd");
  });
  document.addEventListener("pointerdown", () => document.documentElement.classList.remove("kbd"));
  document.addEventListener("keydown", (e) => {
    if (S.phase !== "placing" || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "r" || e.key === "R") setVertical(!S.vertical);
    if (e.key === "Escape" && S.selected !== null && S.fleet[S.selected] !== null) pick(S.selected);
  });
  $("dir-h").addEventListener("click", () => setVertical(false));
  $("dir-v").addEventListener("click", () => setVertical(true));
  $("random").addEventListener("click", randomFleet);
  $("clear").addEventListener("click", clearFleet);
  $("start").addEventListener("click", startBattle);
  $("resign").addEventListener("click", () => finish(false));
  $("new-game").addEventListener("click", newGame);
  $("heat").addEventListener("change", render);
  try {
    E = await loadEngine();
    newGame();
  } catch (err) {
    const box = el("div", { className: "error", textContent: `The engine could not start: ${(err as Error).message}.` });
    $("msg").replaceWith(box);
  }
}
