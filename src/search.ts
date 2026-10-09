/**
 * The grid's search field: reading what was typed into terms the library
 * looks for (library.rs does the looking), and offering what there is to
 * find as it is typed.
 *
 * Each word has to match: a camera, lens, film stock or file name that
 * contains it, or, for a year, a month or a date, when the photo was taken.
 * A word can be kept to one thing (`camera:fm2`, `lens:50mm`, `film:portra`,
 * `date:2026-10`, `name:img_01`), and quotes keep words together and match
 * the whole name: `camera:"Canon EOS R5"` leaves out the R50.
 */

/** The order the grid shows photos in. Imports and Recently Deleted keep their own. */
export type Sort = "newest" | "oldest" | "imported" | "name";

export const SORTS: { value: Sort; label: string }[] = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "imported", label: "Last imported first" },
  { value: "name", label: "File name" },
];

/** Where a term's text is looked for: "any" is all of them. */
export type SearchField = "any" | "camera" | "lens" | "film" | "name";

/**
 * One part of a search. A photo matches when the text is found in the field,
 * or when it was taken from `from` until `until` (compared as text with its
 * `YYYY-MM-DDTHH:MM:SS`), or in `month` of any year. A term asking for none of
 * these matches nothing.
 */
export interface SearchTerm {
  field: SearchField;
  text: string | null;
  /** The whole value must be the text, rather than contain it. */
  exact: boolean;
  from: string | null;
  until: string | null;
  month: number | null;
}

export interface Search {
  terms: SearchTerm[];
}

/** One value the photos in view have, and how many have it. */
export interface Facet {
  value: string;
  count: number;
}

/** What there is to search for in a view. Months are `YYYY-MM`, the latest first. */
export interface SearchFacets {
  cameras: Facet[];
  lenses: Facet[];
  films: Facet[];
  months: Facet[];
}

type Key = SearchField | "date";

const KEYS: Record<string, Key> = {
  camera: "camera",
  lens: "lens",
  film: "film",
  stock: "film",
  date: "date",
  taken: "date",
  name: "name",
  file: "name",
};

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

interface Token {
  key: Key | null;
  value: string;
  quoted: boolean;
  /** A quote was opened and not closed yet: still being typed. */
  open: boolean;
  start: number;
  end: number;
  /** The token as typed. */
  raw: string;
}

/** Splits what was typed into words, `key:value`s and quoted phrases. */
function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  let at = 0;
  while (at < text.length) {
    if (/\s/.test(text[at])) {
      at++;
      continue;
    }
    const start = at;
    let key: Key | null = null;
    const prefix = /^([a-z]+):/i.exec(text.slice(at));
    if (prefix && KEYS[prefix[1].toLowerCase()]) {
      key = KEYS[prefix[1].toLowerCase()];
      at += prefix[0].length;
    }
    let value: string;
    let quoted = false;
    let open = false;
    if (text[at] === '"') {
      quoted = true;
      const close = text.indexOf('"', at + 1);
      open = close < 0;
      const end = open ? text.length : close;
      value = text.slice(at + 1, end);
      at = open ? end : end + 1;
    } else {
      const space = text.slice(at).search(/\s/);
      const end = space < 0 ? text.length : at + space;
      value = text.slice(at, end);
      at = end;
    }
    tokens.push({ key, value, quoted, open, start, end: at, raw: text.slice(start, at) });
  }
  return tokens;
}

const pad = (n: number) => String(n).padStart(2, "0");

interface When {
  from: string | null;
  until: string | null;
  month: number | null;
}

/** The time a year, month or day starts, and the time the next one does. */
function span(year: number, month: number | null, day: number | null): { from: string; until: string } | null {
  if (year < 1800 || year > 2999) return null;
  if (month === null) return { from: String(year), until: String(year + 1) };
  if (month < 1 || month > 12) return null;
  if (day === null) {
    const next = month === 12 ? `${year + 1}-01` : `${year}-${pad(month + 1)}`;
    return { from: `${year}-${pad(month)}`, until: next };
  }
  const date = new Date(Date.UTC(year, month - 1, day));
  if (date.getUTCMonth() !== month - 1) return null;
  date.setUTCDate(day + 1);
  const next = `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
  return { from: `${year}-${pad(month)}-${pad(day)}`, until: next };
}

/** A month's number from its name, or the first three or more letters of it. */
function monthNamed(word: string): number | null {
  const lower = word.toLowerCase().replace(/\.$/, "");
  if (lower.length < 3) return null;
  const at = MONTHS.findIndex((name) => name.toLowerCase().startsWith(lower));
  return at < 0 ? null : at + 1;
}

/** A year, month or day written as a date: 2026, 2026-10, 2026-10-08 (or with / or .). */
function isoSpan(text: string): { from: string; until: string } | null {
  const match = /^(\d{4})(?:[-/.](\d{1,2})(?:[-/.](\d{1,2}))?)?$/.exec(text);
  if (!match) return null;
  return span(Number(match[1]), match[2] ? Number(match[2]) : null, match[3] ? Number(match[3]) : null);
}

/**
 * When a date written in words or numbers means: a year, a month of a year
 * ("October 2026", "oct 2026", 2026-10), a day, a month of any year
 * ("october"), or a span between two of them (`2026-06..2026-08`, either end
 * left open). Null for anything else.
 */
export function parseDate(text: string): When | null {
  const trimmed = text.trim();
  if (trimmed.includes("..")) {
    const [start, end] = trimmed.split("..", 2).map((side) => side.trim());
    const from = start ? isoSpan(start) : null;
    const until = end ? isoSpan(end) : null;
    if ((start && !from) || (end && !until) || (!from && !until)) return null;
    return { from: from?.from ?? null, until: until?.until ?? null, month: null };
  }
  const iso = isoSpan(trimmed);
  if (iso) return { ...iso, month: null };
  const words = /^([a-z]+\.?)(?:,?\s+(\d{4}))?$/i.exec(trimmed);
  const month = words ? monthNamed(words[1]) : null;
  if (!words || month === null) return null;
  if (!words[2]) return { from: null, until: null, month };
  const of = span(Number(words[2]), month, null);
  return of && { ...of, month: null };
}

/** Reads what was typed in the search field into the terms a photo has to match. */
export function parseSearch(text: string): Search {
  const terms: SearchTerm[] = [];
  for (const token of tokenize(text)) {
    const value = token.value.trim();
    if (value === "") continue;
    const nothing = { text: null, exact: false, from: null, until: null, month: null };
    if (token.key === "date") {
      terms.push({ field: "any", ...nothing, ...parseDate(value) });
    } else if (token.key !== null) {
      terms.push({ field: token.key, ...nothing, text: value, exact: token.quoted });
    } else {
      terms.push({ field: "any", ...nothing, text: value, exact: token.quoted, ...parseDate(value) });
    }
  }
  return { terms };
}

/** Whether the search asks for anything; one that doesn't shows every photo. */
export const searching = (text: string) => parseSearch(text).terms.length > 0;

export type SuggestionKind = "camera" | "lens" | "film" | "date";

/** Something the search could look for, offered under the field. */
export interface Suggestion {
  kind: SuggestionKind;
  label: string;
  count: number;
  /** What choosing it puts in the field. */
  insert: string;
}

export const KIND_LABELS: Record<SuggestionKind, string> = {
  camera: "Camera",
  lens: "Lens",
  film: "Film",
  date: "Date",
};

/** The word being typed: where it starts, and what it is so far. */
function currentWord(text: string): { start: number; key: Key | null; value: string } {
  const last = tokenize(text).at(-1);
  const finished = !last || last.end < text.length || (/\s$/.test(text) && !last.open) || (last.quoted && !last.open);
  if (finished) return { start: text.length, key: null, value: "" };
  return { start: last.start, key: last.key, value: last.value.trim() };
}

/** A suggestion, with other text it is found by: a month's `2026-10`. */
interface Candidate {
  suggestion: Suggestion;
  also: string;
}

/** Years and months named as people say them: "2026" and "October 2026". */
function dateCandidates(months: Facet[]): Candidate[] {
  const years = new Map<string, number>();
  for (const month of months) years.set(month.value.slice(0, 4), (years.get(month.value.slice(0, 4)) ?? 0) + month.count);
  const whole = [...years].map(([year, count]) => ({
    suggestion: { kind: "date" as const, label: year, count, insert: `date:${year}` },
    also: year,
  }));
  const named = months.map((month) => {
    const [year, number] = month.value.split("-");
    const label = `${MONTHS[Number(number) - 1] ?? number} ${year}`;
    return { suggestion: { kind: "date" as const, label, count: month.count, insert: `date:${month.value}` }, also: month.value };
  });
  return [...whole, ...named];
}

const quote = (value: string) => `"${value.replace(/"/g, "")}"`;

/**
 * What to offer for the word being typed: cameras, lenses, film stocks and
 * dates whose names hold it, the best matches first. With nothing typed yet,
 * a few of each, to show what can be searched for.
 */
export function suggest(text: string, facets: SearchFacets, limit = 8): Suggestion[] {
  const word = currentWord(text);
  if (word.key === "name") return [];
  const typed = word.value.toLowerCase();
  const used = new Set(
    tokenize(text)
      .filter((token) => !token.open)
      .map((token) => token.raw.toLowerCase()),
  );
  const named = (kind: SuggestionKind, list: Facet[]): Candidate[] =>
    list.map((facet) => ({
      suggestion: { kind, label: facet.value, count: facet.count, insert: `${kind}:${quote(facet.value)}` },
      also: "",
    }));
  const groups = [
    named("camera", facets.cameras),
    named("film", facets.films),
    named("lens", facets.lenses),
    dateCandidates(facets.months),
  ]
    .filter((list) => list.length > 0 && (word.key === null || list[0].suggestion.kind === word.key))
    .map((list) => list.filter((candidate) => !used.has(candidate.suggestion.insert.toLowerCase())));

  if (typed === "") {
    // A few of each kind, to show what there is.
    const each = word.key === null ? 2 : limit;
    return groups.flatMap((list) => list.slice(0, each).map((candidate) => candidate.suggestion)).slice(0, limit);
  }
  const rank = ({ suggestion, also }: Candidate): number => {
    const label = suggestion.label.toLowerCase();
    if (label.startsWith(typed) || also.startsWith(typed)) return 0;
    if (label.split(/[\s\-/]+/).some((part) => part.startsWith(typed))) return 1;
    return label.includes(typed) ? 2 : -1;
  };
  return groups
    .flatMap((list, group) => list.map((candidate, order) => ({ candidate, rank: rank(candidate), group, order })))
    .filter((ranked) => ranked.rank >= 0)
    .sort((a, b) => a.rank - b.rank || a.group - b.group || a.order - b.order)
    .slice(0, limit)
    .map((ranked) => ranked.candidate.suggestion);
}

/** The search with the word being typed replaced by the suggestion, ready for the next word. */
export function applySuggestion(text: string, suggestion: Suggestion): string {
  return `${text.slice(0, currentWord(text).start)}${suggestion.insert} `;
}
