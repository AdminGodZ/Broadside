// The engine, run off the page's thread. A slow move (a tenth take over 60 ms,
// the slowest several hundred, more on a phone) would otherwise freeze the page;
// in a worker the page stays live, and the engine thinks while it shows "aiming".
//
// The engine is the Rust crate broadside-wasm, built to broadside.wasm by
// scripts/prepare.sh: a plain C ABI, numbers in and out (crates/wasm/src/lib.rs).
// Calls go in batches, run in order, one round trip each. Where no worker can
// start, the same batches run on the page.
import wasmUrl from "./broadside.wasm?url";
import EngineWorker from "./engine.worker?worker&inline";

/** The engine's exports, by name. */
export type Fn =
  | "bs_new_game"
  | "bs_set_fleet"
  | "bs_player_fire"
  | "bs_player_won"
  | "bs_engine_fire"
  | "bs_engine_won"
  | "bs_engine_slot"
  | "bs_finish"
  | "bs_update_heat"
  | "bs_heat";
export type Call = [Fn, ...number[]];
export type Batch = Call[];
export type Reply = { results: number[] } | { error: string };

export interface Engine {
  /** Runs the calls in order, after any batch sent before; resolves to their results. */
  run(batch: Batch): Promise<number[]>;
}

const run = (exports: WebAssembly.Exports, batch: Batch) =>
  batch.map(([f, ...args]) => (exports[f] as (...a: number[]) => number)(...args));

/** The next frame, or 0.1 s where no frames are drawn (a hidden tab, a page prerendered ahead). */
const afterPaint = () =>
  new Promise<void>((done) => {
    requestAnimationFrame(() => setTimeout(done));
    setTimeout(done, 100);
  });

/** The engine compiled: from the page in the offline file (scripts/offline.mjs), else fetched. */
async function compile(): Promise<WebAssembly.Module> {
  const inline = (window as { BROADSIDE_WASM?: string }).BROADSIDE_WASM;
  if (inline) return WebAssembly.compile(Uint8Array.from(atob(inline), (ch) => ch.charCodeAt(0)));
  // Placing the fleet doesn't need the engine, only starting the battle does: it downloads once
  // the page is complete and drawn, so the font and the script, which the first view needs,
  // don't share the connection with it.
  if (document.readyState !== "complete") await new Promise((done) => addEventListener("load", done, { once: true }));
  await afterPaint();
  // Compiled while it downloads. A server that sends the wrong content type makes that
  // fail; then the bytes come from the cache.
  try {
    return await WebAssembly.compileStreaming(fetch(wasmUrl));
  } catch {
    return WebAssembly.compile(await (await fetch(wasmUrl)).arrayBuffer());
  }
}

/** The engine in a worker, once it has started there. */
function inWorker(module: WebAssembly.Module): Promise<Engine> {
  return new Promise((ready, fail) => {
    const worker = new EngineWorker();
    const waiting: { done: (r: number[]) => void; fail: (e: Error) => void }[] = [];
    // The first reply says the engine is up; each after it answers the oldest batch.
    waiting.push({ done: () => ready(engine), fail });
    worker.addEventListener("message", ({ data }: MessageEvent<Reply>) => {
      const next = waiting.shift()!;
      if ("error" in data) next.fail(new Error(data.error));
      else next.done(data.results);
    });
    // The worker failed to load (blocked, say), or died: nothing more will come back.
    let dead: Error | undefined;
    worker.addEventListener("error", (e) => {
      e.preventDefault();
      dead = new Error(e.message || "the engine's worker stopped");
      for (const w of waiting.splice(0)) w.fail(dead);
    });
    const engine: Engine = {
      run: (batch) =>
        new Promise((done, fail) => {
          if (dead) return fail(dead);
          waiting.push({ done, fail });
          worker.postMessage(batch);
        }),
    };
    worker.postMessage(module);
  });
}

/** The engine on the page's own thread. Each batch waits for the page to paint first. */
async function onPage(module: WebAssembly.Module): Promise<Engine> {
  const { exports } = await WebAssembly.instantiate(module, {});
  let queue = Promise.resolve();
  return {
    run(batch) {
      const result = queue.then(afterPaint).then(() => run(exports, batch));
      queue = result.then(
        () => {},
        () => {},
      );
      return result;
    },
  };
}

export async function loadEngine(): Promise<Engine> {
  const module = await compile();
  try {
    return await inWorker(module);
  } catch {
    return onPage(module);
  }
}
