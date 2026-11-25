// Fuzzy matching for the command palette: the query's letters must appear in order, and a
// match scores higher when they start words ("ef" → "Export Folder") or run together.

export interface Match {
  score: number;
  /** Indices into the text of the matched characters, for highlighting. */
  hits: number[];
}

const START = 8; // the text's first character
const WORD = 6; // the first character of a word
const RUN = 5; // follows the previous match directly
const GAP = 0.15; // per skipped character

function wordStart(text: string, i: number): boolean {
  if (i === 0) return true;
  const prev = text[i - 1];
  const c = text[i];
  if (!/[\p{L}\p{N}]/u.test(prev)) return true;
  return prev === prev.toLowerCase() && c !== c.toLowerCase(); // camelCase
}

/** Match `query` against `text`, ignoring case. `null` when the letters aren't all there. */
export function fuzzy(query: string, text: string): Match | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  const m = q.length;
  const n = t.length;
  if (!m) return { score: 0, hits: [] };
  if (m > n) return null;

  // best[i][j]: the best score with q[i] matched at t[j]; from[i][j] where q[i-1] was.
  const best: number[][] = [];
  const from: number[][] = [];
  for (let i = 0; i < m; i++) {
    best.push(new Array(n).fill(-Infinity));
    from.push(new Array(n).fill(-1));
    for (let j = i; j < n; j++) {
      if (t[j] !== q[i]) continue;
      const own = 1 + (j === 0 ? START : 0) + (wordStart(text, j) ? WORD : 0);
      if (i === 0) {
        best[i][j] = own - GAP * j;
        continue;
      }
      for (let k = i - 1; k < j; k++) {
        const prev = best[i - 1][k];
        if (prev === -Infinity) continue;
        const s = prev + own + (k === j - 1 ? RUN : -GAP * (j - k - 1));
        if (s > best[i][j]) {
          best[i][j] = s;
          from[i][j] = k;
        }
      }
    }
  }

  let end = -1;
  for (let j = 0; j < n; j++)
    if (best[m - 1][j] > (end < 0 ? -Infinity : best[m - 1][end])) end = j;
  if (end < 0 || best[m - 1][end] === -Infinity) return null;
  const hits: number[] = [];
  for (let i = m - 1, j = end; i >= 0; j = from[i][j], i--) hits.unshift(j);
  return { score: best[m - 1][end], hits };
}
