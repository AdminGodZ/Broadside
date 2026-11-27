// The browser game's logic and drawing. The engine (engine.ts) runs in a worker,
// so every call to it is a batch answered later. Cells are row * 10 + col;
// placement slots are top-left cell * 2 + vertical; outcomes 0 miss, 1 hit,
// 2 + ship for a sinking.
import { type Batch, type Call, type Engine, loadEngine } from "./engine";
import { SHIPS, TEXT, cellName as name } from "./shared";

type Phase = "loading" | "placing" | "battle" | "over";
interface Side { shots: Map<number, number>; sunk: number[]; count: number; last: number | null; fresh: number | null }
interface Chart { cells: HTMLButtonElement[]; layer: HTMLDivElement }

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
/** A random legal fleet, drawn as the engine's Layout::random does (uniform over all fleets), so it needs no engine. */
function randomSlots(): number[] {
  for (;;) {
    const taken = new Set<number>();
    let clear = true;
    const slots = SHIPS.map(([, len]) => {
      let slot: number, cells: number[] | null;
      do {
        slot = Math.floor(Math.random() * 200);
        cells = cellsOf(len, slot);
      } while (!cells);
      for (const c of cells) {
        if (taken.has(c)) clear = false;
        taken.add(c);
      }
      return slot;
    });
    if (clear) return slots;
  }
}
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

/** A chart from the page's markup (Chart.astro), with its cells wired up. */
function chart(root: HTMLElement, onCell: (i: number | null, kind: "click" | "enter" | "leave") => void): Chart {
  const sea = root.querySelector<HTMLDivElement>(".sea")!;
  const cells = [...sea.querySelectorAll<HTMLButtonElement>(".cell")];
  cells.forEach((b, i) => {
    b.addEventListener("click", () => onCell(i, "click"));
    b.addEventListener("pointerenter", () => onCell(i, "enter"));
    b.addEventListener("focus", () => onCell(i, "enter"));
  });
  sea.addEventListener("pointerleave", () => onCell(null, "leave"));
  return { cells, layer: sea.querySelector<HTMLDivElement>(".layer")! };
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
  S.fleet = randomSlots();
  S.selected = null;
  S.fresh = "all";
  note(TEXT.random);
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
/** Runs calls on the engine. Should it fail, the game stops there with the reason shown. */
const call = (batch: Batch) =>
  E.run(batch).catch((err: Error) => {
    stopped(`The engine stopped: ${err.message}.`);
    return new Promise<never>(() => {});
  });
/** Counts games, so an answer that comes back after its game was left (resigned, say) is dropped. */
let game = 0;
const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));

// The engine's belief about your fleet, shown when "Show where the engine expects your ships" is
// ticked. It changes only when the engine fires or learns, so it's fetched then, with that call.
const HEAT: Batch = [["bs_update_heat"], ...Array.from({ length: 100 }, (_, i): Call => ["bs_heat", i])];
let heat: number[] | null = null;
const wantHeat = () => $<HTMLInputElement>("heat").checked && (S.phase === "battle" || S.phase === "over");
/** `batch`, and the heat map after it when it's shown; `took` applies the results. */
function andHeat(batch: Batch) {
  const withHeat = wantHeat();
  return {
    batch: withHeat ? [...batch, ...HEAT] : batch,
    took: (results: number[]) => (heat = withHeat ? results.slice(batch.length + 1) : null),
  };
}
function refreshHeat() {
  if (!wantHeat() || !E) return;
  const g = game, { batch, took } = andHeat([]);
  call(batch).then((results) => {
    if (g !== game) return;
    took(results);
    render();
  });
}

let starting = false;
async function startBattle() {
  if (S.phase !== "placing" || starting) return;
  starting = true;
  const [ok] = await call([["bs_set_fleet", ...(S.fleet as number[])]]);
  starting = false;
  if (S.phase !== "placing") return;
  if (!ok) return flash("Every ship needs a place on the chart.");
  try {
    localStorage.setItem("broadside.fleet", JSON.stringify(S.fleet));
  } catch {}
  S.phase = "battle";
  S.selected = null;
  S.hover = null;
  S.yourTurn = Math.random() < 0.5;
  log(S.yourTurn ? "Coin toss: you fire first" : "Coin toss: the engine fires first", "");
  render();
  if (S.yourTurn) refreshHeat();
  else engineMove();
}
async function playerFire(cell: number) {
  if (S.phase !== "battle" || !S.yourTurn || S.thinking || S.you.shots.has(cell)) return;
  S.yourTurn = false; // one shot at a time
  const g = game;
  const [o, won] = await call([["bs_player_fire", cell], ["bs_player_won"]]);
  if (g !== game || S.phase !== "battle") return;
  if (o < 0) {
    S.yourTurn = true;
    return;
  }
  S.you.shots.set(cell, o);
  S.you.count++;
  S.you.fresh = cell;
  if (o >= 2) S.you.sunk.push(o - 2);
  log(`${name(cell)} ${o >= 2 ? SHIPS[o - 2][0] + " sunk" : OUT(o)}`, "you " + (o === 1 ? "h" : o >= 2 ? "s" : ""));
  if (won) return finish(true);
  render();
  engineMove();
}
async function engineMove() {
  S.thinking = true;
  render();
  // The engine thinks while the page shows it aiming: its shot lands after 0.22 s, or once
  // it has decided, whichever is later.
  const g = game, { batch, took } = andHeat([["bs_engine_fire"], ["bs_engine_won"]]);
  const [results] = await Promise.all([call(batch), wait(220)]);
  if (g !== game || S.phase !== "battle") return;
  const [r, won] = results, cell = r >> 4, o = r & 15;
  took(results);
  S.it.shots.set(cell, o);
  S.it.count++;
  S.it.last = cell;
  if (o >= 2) S.it.sunk.push(o - 2);
  log(`${name(cell)} ${o >= 2 ? SHIPS[o - 2][0] + " sunk" : OUT(o)}`, "it " + (o === 1 ? "h" : o >= 2 ? "s" : ""));
  S.thinking = false;
  if (won) return finish(false);
  S.yourTurn = true;
  render();
}
async function finish(youWon: boolean) {
  S.phase = "over";
  S.thinking = false;
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
  // Its fleet, for the reveal; then both sides of the engine learn from the game.
  const g = game, { batch, took } = andHeat([...SHIPS.map((_, s): Call => ["bs_engine_slot", s]), ["bs_finish"]]);
  const results = await call(batch);
  if (g !== game) return;
  S.reveal = results.slice(0, SHIPS.length);
  took(results);
  render();
}
/** The fleet the player last went into battle with, if any. Game.astro checks it the same way before the first paint. */
function savedFleet(): number[] | null {
  let saved: unknown = null;
  try {
    saved = JSON.parse(localStorage.getItem("broadside.fleet") ?? "null");
  } catch {}
  return Array.isArray(saved) && saved.length === 5 && saved.every((x) => Number.isInteger(x)) ? saved : null;
}
function newGame() {
  game++;
  heat = null;
  call([["bs_new_game", Math.random() * 2 ** 52]]);
  Object.assign(S, { phase: "placing", selected: null, hover: null, reveal: null, thinking: false, fresh: null, you: newSide(), it: newSide() });
  $("log").replaceChildren();
  $("banner").hidden = true;
  const saved = savedFleet();
  if (saved) {
    S.fleet = saved;
    note(TEXT.saved);
  } else randomFleet();
  render();
}

// ---------- view ----------
let flashTimer: ReturnType<typeof setTimeout> | undefined;
let notice: string | null = null;
const note = (text: string | null) => (notice = text);
/** The status line: a heading and a line under it. Left as it is when they haven't changed,
 * so the line Game.astro wrote before the first paint stays (redrawn, it would count as a late paint). */
function say(head: string, line: string) {
  const msg = $("msg");
  if (!msg) return; // the engine stopped, and says so in its place
  if (msg.childNodes.length === 2 && msg.textContent === head + line) return;
  msg.replaceChildren(head, el("span", { className: "msg-sub", textContent: line }));
}
function flash(text: string) {
  say(text, KEYS ? "Try another cell, or turn it with R." : "Try another cell, or turn it.");
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
      : TEXT.pickHint;
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
  const showHeat = heatOn && !placing && heat !== null;
  home.cells.forEach((b, i) => {
    const o = S.it.shots.get(i);
    b.className = "cell" + (o === undefined ? "" : o === 0 ? " miss" : " hit") + (S.it.last === i ? " fresh" : "");
    // Not disabled in battle (a disabled cell gets no hover, so no figure); just out of the tab order.
    b.tabIndex = placing ? 0 : -1;
    b.replaceChildren();
    let chance = "";
    if (showHeat && o === undefined) {
      const h = heat![i];
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
  $("resign").hidden = S.phase !== "battle";
  $("new-game").hidden = S.phase !== "over";
  if (placing) {
    const ready = S.fleet.every((x) => x !== null);
    if (S.selected !== null) {
      const [n, len] = SHIPS[S.selected];
      const turn = KEYS ? "Press R to turn it." : "Turn it with Across or Down.";
      say(S.fleet[S.selected] === null ? `Place your ${n}` : `Move your ${n}`, notice ?? `${len} cells, ${S.vertical ? "down" : "across"}. ${turn}`);
    } else say(ready ? TEXT.ready : "Place your fleet", notice ?? TEXT.moveOn);
    notice = null;
  } else if (S.phase === "battle") {
    if (S.thinking) say("The engine is aiming…", "It weighs every fleet that could be hiding.");
    else say("Your shot", "Pick a cell in enemy waters.");
  } else if (S.phase === "over") {
    say("Game over", "Your fleet stays where it was for the next game.");
  }
  $("rec-w").textContent = String(S.record.w);
  $("rec-l").textContent = String(S.record.l);
  const ws = S.record.winShots;
  $("rec-avg").textContent = ws.length ? (ws.reduce((a, b) => a + b, 0) / ws.length).toFixed(1) : "–";
}

// ---------- boot ----------
export function start() {
  const engine = loadEngine();
  // Start battle needs the engine; pressed before it is there, it waits for it.
  const withEngine = (f: () => void) => () => (E ? f() : engine.then(f, () => {}));
  target = chart($("target"), (i, kind) => {
    if (kind === "click" && i !== null) playerFire(i);
  });
  home = chart($("home"), (i, kind) => {
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
  $("start").addEventListener("click", withEngine(startBattle));
  $("resign").addEventListener("click", () => finish(false));
  $("new-game").addEventListener("click", newGame);
  $("heat").addEventListener("change", () => {
    refreshHeat();
    render();
  });

  // Placing needs no engine: the saved fleet, or a random one, is on the chart now.
  const saved = savedFleet();
  S.phase = "placing";
  if (saved) {
    S.fleet = saved;
    note(TEXT.saved);
    render();
  } else randomFleet();
  engine.then(
    (e) => {
      E = e;
      if (S.phase === "placing") call([["bs_new_game", Math.random() * 2 ** 52]]); // keep the fleet, and any move already made
      else newGame();
    },
    (err: Error) => stopped(`The engine could not start: ${err.message}.`),
  );
}
/** The engine is out: the game stops, with the reason in place of the status line. */
function stopped(why: string) {
  game++;
  S.phase = "loading";
  $("dock").hidden = true;
  $("msg")?.replaceWith(el("div", { className: "error", textContent: why }));
}
