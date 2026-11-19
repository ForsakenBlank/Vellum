// fuzzy matching for the command palette and settings search. every query character must
// appear in order. matches score higher at the start of the text, at the start of words and
// when characters run together, so "tdk" finds "Theme: dark" and "exp" finds "Export".

export interface FuzzyMatch {
  score: number;
  /** indexes of the matched characters in the text, for highlighting */
  positions: number[];
}

function isBoundary(text: string, index: number): boolean {
  if (index === 0) return true;
  const before = text[index - 1] ?? "";
  const here = text[index] ?? "";
  if (/[\s\-_./:,()]/.test(before)) return true;
  // camelCase and digits after letters count as the start of a word
  return /[a-z]/.test(before) && /[A-Z0-9]/.test(here);
}

/** matches one word of a query against one text, trying every start to find the best */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (q.length === 0) return { score: 0, positions: [] };
  if (q.length > t.length) return null;

  let best: FuzzyMatch | null = null;
  let start = t.indexOf(q.charAt(0));
  while (start !== -1) {
    const match = matchFrom(q, t, text, start);
    if (match && (!best || match.score > best.score)) best = match;
    start = t.indexOf(q.charAt(0), start + 1);
  }
  return best;
}

function matchFrom(q: string, t: string, original: string, start: number): FuzzyMatch | null {
  const positions: number[] = [];
  let score = 0;
  let ti = start;
  let previous = -2;
  for (const ch of q) {
    // prefer the next word start that holds this character over the nearest occurrence
    let found = -1;
    for (let i = ti; i < t.length; i++) {
      if (t[i] !== ch) continue;
      if (found === -1) found = i;
      if (i === previous + 1) {
        found = i;
        break;
      }
      if (isBoundary(original, i)) {
        found = i;
        break;
      }
    }
    if (found === -1) return null;

    score += 1;
    if (found === previous + 1) score += 6;
    if (isBoundary(original, found)) score += 8;
    if (found === 0) score += 6;
    if (previous >= 0) score -= Math.min(found - previous - 1, 6) * 0.4;
    positions.push(found);
    previous = found;
    ti = found + 1;
  }
  // a whole word match beats a scattered one, and shorter texts beat longer ones
  if (t.includes(q)) score += 10;
  score -= t.length * 0.02;
  return { score, positions };
}

/**
 * keywords are long, so a scattered fuzzy match there finds nonsense. they need the word
 * itself, scoring more when it starts a word.
 */
function keywordMatch(word: string, keywords: string): { score: number } | null {
  const text = keywords.toLowerCase();
  const at = text.indexOf(word.toLowerCase());
  if (at === -1) return null;
  const starts = at === 0 || /[\s\-_./:,()]/.test(text.charAt(at - 1));
  return { score: (starts ? 14 : 6) + word.length };
}

export interface Searchable {
  title: string;
  /** extra words that should find this item but are not shown */
  keywords?: string | undefined;
}

export interface Ranked<T> {
  item: T;
  score: number;
  /** positions in the title to highlight */
  positions: number[];
}

/**
 * ranks items for a query of one or more words. every word has to match the title or the
 * keywords; title matches count for more. items keep their order when scores tie.
 */
export function rank<T>(query: string, items: T[], fields: (item: T) => Searchable): Ranked<T>[] {
  const words = query.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) {
    return items.map((item) => ({ item, score: 0, positions: [] }));
  }
  const ranked: (Ranked<T> & { order: number })[] = [];
  items.forEach((item, order) => {
    const { title, keywords = "" } = fields(item);
    let score = 0;
    const positions = new Set<number>();
    for (const word of words) {
      const inTitle = fuzzyMatch(word, title);
      const inKeywords = keywords ? keywordMatch(word, keywords) : null;
      if (inTitle && (!inKeywords || inTitle.score + 2 >= inKeywords.score * 0.6)) {
        score += inTitle.score;
        inTitle.positions.forEach((p) => positions.add(p));
      } else if (inKeywords) {
        score += inKeywords.score * 0.6;
      } else {
        return;
      }
    }
    ranked.push({ item, score, positions: [...positions].sort((a, b) => a - b), order });
  });
  ranked.sort((a, b) => b.score - a.score || a.order - b.order);
  return ranked.map(({ item, score, positions }) => ({ item, score, positions }));
}
