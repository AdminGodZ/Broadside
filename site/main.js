// Broadside website: the hero replay and the download button.

const SVG = "http://www.w3.org/2000/svg";
const SHIP_NAMES = ["Carrier", "Battleship", "Cruiser", "Submarine", "Destroyer"];
const SHOT_MS = 420;   // time between shots
const HOLD_MS = 3200;  // how long a finished game stays on the board

function el(name, attrs, parent) {
  const node = document.createElementNS(SVG, name);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  if (parent) parent.appendChild(node);
  return node;
}

// Cell index = row * 10 + col; rows are A-J, columns 1-10 (as in the engine).
const cx = (cell) => (cell % 10) * 10 + 5;
const cy = (cell) => Math.floor(cell / 10) * 10 + 5;

function drawSea(svg) {
  el("rect", { class: "sea", x: 0, y: 0, width: 100, height: 100, rx: 1 }, svg);
  for (let i = 1; i < 10; i++) {
    el("line", { class: "grid", x1: i * 10, y1: 0, x2: i * 10, y2: 100 }, svg);
    el("line", { class: "grid", x1: 0, y1: i * 10, x2: 100, y2: i * 10 }, svg);
  }
  for (let i = 0; i < 10; i++) {
    el("text", { class: "coord", x: i * 10 + 5, y: -4 }, svg).textContent = String(i + 1);
    el("text", { class: "coord", x: -4, y: i * 10 + 5 }, svg).textContent = "ABCDEFGHIJ"[i];
  }
  const ships = el("g", { class: "ships" }, svg);
  const shots = el("g", { class: "shots" }, svg);
  return { ships, shots };
}

function drawShip(layer, cells) {
  const xs = cells.map((c) => c % 10), ys = cells.map((c) => Math.floor(c / 10));
  const x = Math.min(...xs) * 10, y = Math.min(...ys) * 10;
  const w = (Math.max(...xs) + 1) * 10 - x, h = (Math.max(...ys) + 1) * 10 - y;
  el("rect", { class: "ship", x: x + 1.6, y: y + 1.6, width: w - 3.2, height: h - 3.2, rx: 3.4 }, layer);
}

function drawShot(layer, cell, hit) {
  const x = cx(cell), y = cy(cell);
  if (hit) {
    const g = el("g", { class: "hit" }, layer);
    el("line", { x1: x - 2.4, y1: y - 2.4, x2: x + 2.4, y2: y + 2.4 }, g);
    el("line", { x1: x - 2.4, y1: y + 2.4, x2: x + 2.4, y2: y - 2.4 }, g);
  } else {
    el("circle", { class: "miss", cx: x, cy: y, r: 1.2 }, layer);
  }
}

function sunkText(n) {
  return n === 0 ? "" : n === 1 ? ", 1 ship sunk" : `, ${n} ships sunk`;
}

async function startReplay() {
  const svg = document.getElementById("board");
  const status = document.getElementById("replay-status");
  if (!svg) return;
  const { ships, shots } = drawSea(svg);

  let games;
  try {
    games = await (await fetch("/games.json")).json();
  } catch {
    return; // the empty board still reads as a board
  }

  // One game, played from shot 0 to `upto` (all of it when upto is omitted).
  function render(game, upto = game.shots.length) {
    ships.replaceChildren();
    shots.replaceChildren();
    const fired = new Set(game.shots.slice(0, upto));
    let sunk = 0;
    game.ships.forEach((cells) => {
      if (cells.every((c) => fired.has(c))) { drawShip(ships, cells); sunk++; }
    });
    const occupied = new Set(game.ships.flat());
    game.shots.slice(0, upto).forEach((c) => drawShot(shots, c, occupied.has(c)));
    return sunk;
  }

  if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
    render(games[0]);
    status.textContent = `Fleet sunk in ${games[0].shots.length} shots`;
    return;
  }

  // Only animate while the board is on screen and the tab is visible.
  let visible = true;
  new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(svg);
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const running = async () => { while (!visible || document.hidden) await wait(400); };

  for (let g = 0; ; g = (g + 1) % games.length) {
    const game = games[g];
    shots.classList.remove("fading");
    ships.classList.remove("fading");
    render(game, 0);
    status.textContent = "Shot 0";
    for (let i = 1; i <= game.shots.length; i++) {
      await wait(SHOT_MS);
      await running();
      const sunk = render(game, i);
      const cell = game.shots[i - 1];
      el("circle", { class: "latest", cx: cx(cell), cy: cy(cell), r: 4 }, shots);
      const last = game.ships.find((cells) => cells.includes(cell));
      const justSunk = last && last.every((c) => game.shots.slice(0, i).includes(c));
      status.textContent = sunk === 5 ? `Fleet sunk in ${i} shots`
        : justSunk ? `Shot ${i}: ${SHIP_NAMES[game.ships.indexOf(last)]} sunk`
        : `Shot ${i}${sunkText(sunk)}`;
    }
    await wait(HOLD_MS);
    shots.classList.add("fading");
    ships.classList.add("fading");
    await wait(600);
  }
}

// Point the download button at the build for this computer when the browser says which it is.
function pickDownload() {
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  let os = null, label = null;
  if (/Win/i.test(platform) || /Windows/i.test(ua)) { os = "windows"; label = "Download for Windows"; }
  else if (/Mac/i.test(platform) && !/iPhone|iPad/i.test(ua)) { os = "mac-arm"; label = "Download for Mac"; }
  else if (/Linux/i.test(platform) && !/Android/i.test(ua)) {
    os = /aarch64|arm/i.test(platform + ua) ? "linux-arm" : "linux";
    label = "Download for Linux";
  }
  if (!os) return;
  const link = document.querySelector(`.systems a[data-os="${os}"]`);
  const main = document.getElementById("download-main");
  main.href = link.href;
  main.innerHTML = `${label}<small>${link.textContent}, version ${document.body.dataset.version}</small>`;
  link.parentElement.remove();
}

startReplay();
pickDownload();
