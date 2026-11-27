// The engine's own thread (game.ts starts it, inlined into the page's script).
// It's handed the compiled engine once, then runs batches of calls in the order
// they come and answers each with their results; one batch per message keeps a
// move, or a heat map, to a single round trip.
import type { Batch, Reply } from "./engine";

const post = (reply: Reply) => postMessage(reply);
let engine: Record<string, (...args: number[]) => number> | undefined;

addEventListener("message", async ({ data }: MessageEvent<WebAssembly.Module | Batch>) => {
  try {
    if (data instanceof WebAssembly.Module) {
      engine = (await WebAssembly.instantiate(data, {})).exports as typeof engine;
      post({ results: [] });
    } else post({ results: data.map(([f, ...args]) => engine![f](...args)) });
  } catch (err) {
    post({ error: err instanceof Error ? err.message : String(err) });
  }
});
