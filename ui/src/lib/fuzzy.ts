/**
 * fuzzy matching for the command palette. "exgf" finds "Export to GIF".
 *
 * the query has to appear in order, not necessarily together. matches score
 * higher at the start of words, in runs, and near the start of the text, so
 * the result people mean usually comes first.
 */

export interface FuzzyMatch {
  score: number;
  /** positions in the text that matched, for highlighting */
  positions: number[];
}

function isWordStart(text: string, index: number): boolean {
  if (index === 0) return true;
  const before = text.charAt(index - 1);
  const here = text.charAt(index);
  if (/[\s\-_./:]/.test(before)) return true;
  // camelCase and PascalCase boundaries
  return before === before.toLowerCase() && here !== here.toLowerCase();
}

/** finds the best way to match `query` in `text`, or null if it does not match */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  const needle = query.toLowerCase().replace(/\s+/g, "");
  if (!needle) return { score: 0, positions: [] };
  const hay = text.toLowerCase();
  if (needle.length > hay.length) return null;

  // dynamic programming over (query char, text position): the best score that
  // ends with query char i matched at text position j
  const rows = needle.length;
  const cols = hay.length;
  const none = -Infinity;
  const best: number[][] = [];
  const from: number[][] = [];

  for (let i = 0; i < rows; i++) {
    const row = new Array<number>(cols).fill(none);
    const back = new Array<number>(cols).fill(-1);
    for (let j = i; j < cols; j++) {
      if (hay[j] !== needle[i]) continue;
      let bonus = 1;
      if (isWordStart(text, j)) bonus += 8;
      if (j === 0) bonus += 4;

      if (i === 0) {
        // a little less for every character skipped at the start
        row[j] = bonus - Math.min(j, 10) * 0.2;
        continue;
      }
      const previous = best[i - 1] ?? [];
      let bestPrevious = none;
      let bestAt = -1;
      for (let k = i - 1; k < j; k++) {
        const value = previous[k] ?? none;
        if (value === none) continue;
        const gap = j - k - 1;
        // a run of matches is worth a lot, a gap costs a little
        const step = gap === 0 ? value + 6 : value - Math.min(gap, 8) * 0.3;
        if (step > bestPrevious) {
          bestPrevious = step;
          bestAt = k;
        }
      }
      if (bestAt >= 0) {
        row[j] = bestPrevious + bonus;
        back[j] = bestAt;
      }
    }
    best.push(row);
    from.push(back);
  }

  const last = best[rows - 1] ?? [];
  let score = none;
  let end = -1;
  last.forEach((value, j) => {
    if (value > score) {
      score = value;
      end = j;
    }
  });
  if (end < 0) return null;

  const positions: number[] = [];
  for (let i = rows - 1, j = end; i >= 0 && j >= 0; i--) {
    positions.unshift(j);
    j = from[i]?.[j] ?? -1;
  }
  // shorter texts win ties, "Open" beats "Open recent projects folder"
  return { score: score - cols * 0.01, positions };
}

export interface Searchable {
  /** the text that is shown and highlighted */
  label: string;
  /** extra words that should find it, not shown */
  keywords?: string[];
}

const KEYWORD_PENALTY = 6;

export interface Ranked<T> {
  item: T;
  score: number;
  /** positions in the label, empty when only a keyword matched */
  positions: number[];
}

/** filters and sorts items by how well they match the query */
export function fuzzyRank<T extends Searchable>(query: string, items: T[]): Ranked<T>[] {
  const ranked: Ranked<T>[] = [];
  for (const item of items) {
    const onLabel = fuzzyMatch(query, item.label);
    let best = onLabel ? { score: onLabel.score, positions: onLabel.positions } : null;
    for (const keyword of item.keywords ?? []) {
      const onKeyword = fuzzyMatch(query, keyword);
      // a keyword match counts for less than a match on the label people can see
      if (onKeyword && (!best || onKeyword.score - KEYWORD_PENALTY > best.score)) {
        best = { score: onKeyword.score - KEYWORD_PENALTY, positions: [] };
      }
    }
    if (best) ranked.push({ item, ...best });
  }
  // stable for equal scores, so callers can control the order of ties
  return ranked
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => b.entry.score - a.entry.score || a.index - b.index)
    .map(({ entry }) => entry);
}
