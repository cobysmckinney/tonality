import { listen as tauriListen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

type Listen = <T>(event: string, handler: EventCallback<T>) => Promise<UnlistenFn>;

/**
 * Listens for each backend event, right away. The returned function stops
 * every listener, including ones still being set up.
 */
export function listenAll(
  handlers: Record<string, (payload: never) => void>,
  listen: Listen = tauriListen,
): () => void {
  let stopped = false;
  const unlistens: UnlistenFn[] = [];
  for (const [event, handler] of Object.entries(handlers)) {
    void listen(event, ({ payload }) => {
      if (!stopped) handler(payload as never);
    }).then((unlisten) => {
      if (stopped) unlisten();
      else unlistens.push(unlisten);
    });
  }
  return () => {
    stopped = true;
    for (const unlisten of unlistens.splice(0)) unlisten();
  };
}

/** Runs each task on its own, so one failing doesn't stop the rest. Returns why each failed one did. */
export async function settleEach(tasks: (() => Promise<unknown>)[]): Promise<unknown[]> {
  const results = await Promise.allSettled(tasks.map((task) => task()));
  return results.flatMap((r) => (r.status === "rejected" ? [r.reason] : []));
}
