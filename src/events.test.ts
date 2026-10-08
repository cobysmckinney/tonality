import { expect, test } from "bun:test";
import { listenAll, settleEach } from "./events";

/** A stand-in for Tauri's `listen` that records who listens and when they stop. */
function fakeBackend() {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  const stopped: string[] = [];
  let ready: () => void = () => {};
  const started = new Promise<void>((resolve) => (ready = resolve));
  const listen = async <T,>(name: string, handler: (event: { payload: T }) => void) => {
    handlers.set(name, handler as (event: { payload: unknown }) => void);
    await started;
    return () => void stopped.push(name);
  };
  const emit = (name: string, payload: unknown) => handlers.get(name)?.({ payload });
  return { listen: listen as never, handlers, stopped, ready, emit };
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("every event is listened for straight away", () => {
  const backend = fakeBackend();
  listenAll({ a: () => {}, b: () => {} }, backend.listen);
  expect([...backend.handlers.keys()]).toEqual(["a", "b"]);
});

test("each event reaches its handler", () => {
  const backend = fakeBackend();
  const seen: unknown[] = [];
  listenAll({ a: (p: number) => seen.push(["a", p]), b: (p: string) => seen.push(["b", p]) }, backend.listen);
  backend.emit("b", "x");
  backend.emit("a", 1);
  expect(seen).toEqual([
    ["b", "x"],
    ["a", 1],
  ]);
});

test("stopping after the listeners are set up stops each once", async () => {
  const backend = fakeBackend();
  const stop = listenAll({ a: () => {}, b: () => {} }, backend.listen);
  backend.ready();
  await settle();
  stop();
  stop();
  expect(backend.stopped).toEqual(["a", "b"]);
});

test("stopping before the listeners are set up still stops them, and nothing gets through meanwhile", async () => {
  const backend = fakeBackend();
  const seen: unknown[] = [];
  const stop = listenAll({ a: (p: number) => seen.push(p) }, backend.listen);
  stop();
  backend.emit("a", 1);
  backend.ready();
  await settle();
  expect(backend.stopped).toEqual(["a"]);
  expect(seen).toEqual([]);
});

test("one failing task doesn't stop the others", async () => {
  const done: string[] = [];
  const failures = await settleEach([
    async () => void done.push("first"),
    async () => {
      throw "no presets";
    },
    async () => void done.push("last"),
  ]);
  expect(done).toEqual(["first", "last"]);
  expect(failures).toEqual(["no presets"]);
});
