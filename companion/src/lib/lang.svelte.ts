// The language the phone app speaks, which is the TV's. Text is written in English where it is
// shown, inside t, tc or tn, and looked up in the catalog under companion/locales.

type Catalog = Record<string, string | string[]>;

const shipped = import.meta.glob<{ default: Catalog }>('../../locales/*.po');
const CONTEXT_END = '\u0004';

// The catalog is swapped whole, so only the swap needs to be seen, not every text in it.
let spoken = $state.raw({ tag: 'en', catalog: null as Catalog | null });
/** The language last asked for, so an older load that finishes late does not take its place. */
let wanted = 'en';

/** The tag of the language in use. */
export function language(): string {
  return spoken.tag;
}

function shippedTag(wanted: string): string | null {
  const have = Object.keys(shipped).map((path) => path.slice(path.lastIndexOf('/') + 1, -3));
  const lower = wanted.replace('_', '-').toLowerCase();
  const [lang, ...rest] = lower.split('-');
  const region = rest.at(-1) ?? '';
  const find = (tag: string) => have.find((h) => h.toLowerCase() === tag.toLowerCase()) ?? null;
  if (find(lower)) return find(lower);
  if (lang === 'pt') {
    const european = region !== '' && !['br', 'us', '419'].includes(region);
    const portuguese = find(european ? 'pt-PT' : 'pt-BR');
    if (portuguese) return portuguese;
  }
  return find(lang) ?? have.find((h) => h.split('-')[0].toLowerCase() === lang) ?? null;
}

/**
 * Puts the language `asked` names in use, or English when the app has no such catalog. A catalog
 * that will not load leaves the language as it was, and the next call tries again.
 */
export async function speak(asked: string | null | undefined) {
  const tag = (asked && shippedTag(asked)) || 'en';
  wanted = tag;
  if (tag === spoken.tag) return;
  let catalog: Catalog | null = null;
  if (tag !== 'en') {
    try {
      catalog = (await shipped[`../../locales/${tag}.po`]()).default;
    } catch (e) {
      console.warn('language not loaded', tag, e);
      return;
    }
  }
  if (wanted !== tag) return;
  spoken = { tag, catalog };
  document.documentElement.lang = tag;
}

/** Which of a language's plural forms `n` takes, in the order its catalog lists them. */
export function forms(tag: string, n: number): number {
  n = Math.abs(n);
  const lower = tag.toLowerCase();
  const lang = lower.split('-')[0];
  const tens = n % 10;
  const hundreds = n % 100;
  if (['ja', 'ko', 'zh', 'vi', 'th', 'id', 'ms', 'ka', 'km', 'lo', 'my'].includes(lang)) return 0;
  if (['fr', 'oc', 'fil', 'tl', 'hy', 'ln', 'mg', 'wa'].includes(lang) || lower === 'pt' || lower === 'pt-br' || lower === 'pt_br') return n > 1 ? 1 : 0;
  if (['ru', 'uk', 'be', 'sr', 'hr', 'bs'].includes(lang)) {
    if (tens === 1 && hundreds !== 11) return 0;
    return tens >= 2 && tens <= 4 && !(hundreds >= 12 && hundreds <= 14) ? 1 : 2;
  }
  if (lang === 'pl') {
    if (n === 1) return 0;
    return tens >= 2 && tens <= 4 && !(hundreds >= 12 && hundreds <= 14) ? 1 : 2;
  }
  if (lang === 'cs' || lang === 'sk') return n === 1 ? 0 : n >= 2 && n <= 4 ? 1 : 2;
  if (lang === 'ro') return n === 1 ? 0 : n === 0 || (hundreds >= 1 && hundreds <= 19) ? 1 : 2;
  if (lang === 'lt') {
    if (tens === 1 && hundreds !== 11) return 0;
    return tens >= 2 && !(hundreds >= 11 && hundreds <= 19) ? 1 : 2;
  }
  if (lang === 'lv') return n === 0 ? 2 : tens === 1 && hundreds !== 11 ? 0 : 1;
  if (lang === 'sl') return hundreds === 1 ? 0 : hundreds === 2 ? 1 : hundreds === 3 || hundreds === 4 ? 2 : 3;
  if (lang === 'ar') {
    if (n <= 2) return n;
    return hundreds >= 3 && hundreds <= 10 ? 3 : hundreds >= 11 ? 4 : 5;
  }
  return n === 1 ? 0 : 1;
}

type Values = Record<string, string | number>;

function fill(template: string, values?: Values): string {
  if (!values) return template;
  return template.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (place, name) => (name in values ? String(values[name]) : place));
}

function found(context: string | null, text: string): string | string[] | undefined {
  return spoken.catalog?.[context === null ? text : context + CONTEXT_END + text];
}

/** Text in the language in use. Values fill the `{name}` places: `t('Opened {when}', { when })`. */
export function t(text: string, values?: Values): string {
  const said = found(null, text);
  return fill(typeof said === 'string' ? said : text, values);
}

/** `t` with a context first, for English words that mean two things: `tc('games', 'Play')`. */
export function tc(context: string, text: string, values?: Values): string {
  const said = found(context, text);
  return fill(typeof said === 'string' ? said : text, values);
}

/** Text that counts something, with `{n}` for the number: `tn(count, '{n} app', '{n} apps')`. */
export function tn(n: number, one: string, other: string, values?: Values): string {
  const said = found(null, one);
  const chosen = Array.isArray(said) ? (said[forms(spoken.tag, n)] ?? said[said.length - 1]) : n === 1 ? one : other;
  return fill(chosen, { n, ...values });
}

/** Names text for translators and leaves it as written, for a table whose text goes through `tr` when shown. */
export function mark(text: string): string {
  return text;
}

/** `t` for text held in a table, which `mark` names for translators. */
export function tr(text: string): string {
  return t(text);
}
