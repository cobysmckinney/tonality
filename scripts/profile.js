// Responsiveness profile: drives Edit through common interactions and logs, per scenario, every
// frame gap over 50 ms ("stalls") and the time of each API call.
//
//   TONALITY_LIBRARY=<scratch library> TONALITY_DEV_SCRIPT=scripts/profile.js npm run tauri dev
//
// Use a scratch copy of a library: it adds masks and commits. Needs at least two photos; the
// AI scenarios need the subject model installed (TONALITY_MODELS). Results: lines with [prof].
const until = async (f, ms = 90000) => { const t = Date.now(); while (!f()) { if (Date.now() - t > ms) throw new Error("timeout"); await sleep(50); } };
// ---- instrumentation ----
let scenario = "startup";
const calls = {}; // scenario -> name -> [ms]
const wrap = (obj, prefix) => {
  for (const k of Object.keys(obj)) {
    const v = obj[k];
    if (typeof v === "function") {
      obj[k] = (...a) => {
        const t = performance.now(), sc = scenario;
        const r = v.apply(obj, a);
        const done = () => ((calls[sc] ??= {})[prefix + k] ??= []).push(performance.now() - t);
        if (r && r.then) r.then(done, done); else done();
        return r;
      };
    } else if (v && typeof v === "object") wrap(v, prefix + k + ".");
  }
};
wrap(api, "");
const stalls = {}; // scenario -> [gap ms]
let last = performance.now();
const tick = (t) => { const g = t - last; last = t; if (g > 50) (stalls[scenario] ??= []).push(Math.round(g)); requestAnimationFrame(tick); };
requestAnimationFrame(tick);
const report = (sc) => {
  const c = calls[sc] ?? {};
  const parts = Object.entries(c).map(([k, v]) => { const s = [...v].sort((a, b) => a - b); return `${k} n=${v.length} p50=${Math.round(s[s.length >> 1])} max=${Math.round(s[s.length - 1])}`; });
  const st = stalls[sc] ?? [];
  console.info(`[prof] ${sc}: stalls ${st.length} [${st.slice(0, 12).join(",")}${st.length > 12 ? ",…" : ""}] max=${Math.max(0, ...st)}\n    ${parts.join("\n    ")}`);
};
const run = async (name, f) => { scenario = name; await f(); await sleep(800); report(name); };
const drag = async (set, n = 60) => { for (let i = 0; i < n; i++) { set(i); await new Promise((r) => requestAnimationFrame(r)); } };

await until(() => library.loaded && library.images.length && !library.restoring);
await sleep(1500);
report("startup");

await run("library-grid", async () => { library.setQuery({ minRating: 0 }); await sleep(300); library.setQuery({ sort: "name" }); await sleep(300); library.setQuery({ sort: "captureTime" }); await sleep(500); });

const [plain, truck] = library.images;
await run("open-photo", async () => { library.open(plain.id); await until(() => session.edit && !session.loading && session.image?.id === plain.id); await sleep(1500); });

await run("drag-exposure (no masks)", async () => { await drag((i) => (session.edit.light.exposure = Math.sin(i / 8))); await session.commit("exposure"); });

await run("commits x10", async () => { for (let i = 0; i < 10; i++) { session.edit.light.contrast = i * 5; await session.commit(null); await sleep(60); } await sleep(1500); });

await run("filmstrip next x5", async () => { for (let i = 0; i < 5; i++) { library.step(1); await sleep(350); } await until(() => !session.loading); await sleep(1500); });

await run("open second photo", async () => { library.open(truck.id); await until(() => session.edit && !session.loading && session.image?.id === truck.id); await until(() => [...session.aiStatus.values()].every((s) => s.state !== "working")); await sleep(1500); });

await run("add AI subject mask", async () => {
  if (session.edit.masks?.some((m) => m.shape.kind === "ai")) return;
  session.startMasks(); await session.addAiMask("subject"); session.stopMasks(); await sleep(1000);
});

// Add a brush mask with strokes so mask layers carry real work.
await run("add brush strokes", async () => {
  session.startMasks(); await session.addMask("brush");
  const m = session.edit.masks.at(-1);
  for (let s = 0; s < 15; s++) m.shape.strokes.push({ points: Array.from({ length: 40 }, (_, k) => [0.2 + k * 0.015, 0.3 + s * 0.03]), radius: 0.03, feather: 0.6, strength: 1, erase: false });
  await session.commit(null); session.stopMasks(); await sleep(1000);
});

await run("drag-exposure (brush + AI masks)", async () => { await drag((i) => (session.edit.light.exposure = Math.sin(i / 8))); await session.commit("exposure"); });

await run("drag-lens-distortion (masks)", async () => {
  session.edit.lens = { profile: false, distortion: 100, vignetting: 100, chromatic: true, manualDistortion: 0 };
  await drag((i) => (session.edit.lens.manualDistortion = Math.round(50 * Math.sin(i / 10))), 40); await session.commit(null);
});

await run("drag during AI detect", async () => {
  session.startMasks();
  const p = session.addAiMask("subject");
  await sleep(200);
  await drag((i) => (session.edit.light.shadows = Math.round(40 * Math.sin(i / 8))), 80);
  await p; session.stopMasks();
});

await run("undo x8", async () => { for (let i = 0; i < 8; i++) { await session.undo(); await sleep(40); } await sleep(1200); });

await run("back to library", async () => { app.mode = "library"; await sleep(1500); });
console.info("[prof] DONE");
