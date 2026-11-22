// Turns a gettext catalog into a module the phone app loads when its language is asked for.
import type { Plugin } from 'vite';

/** Separates a context from its text in a catalog key, as gettext does. */
const CONTEXT_END = '\u0004';

function unquote(quoted: string): string {
  return quoted.slice(1, -1).replace(/\\(.)/g, (_, c) => (c === 'n' ? '\n' : c === 't' ? '\t' : c));
}

/** The finished translations in `text`: one string per key, or one per plural form. */
export function readCatalog(text: string): Record<string, string | string[]> {
  const out: Record<string, string | string[]> = {};
  let entry = { context: null as string | null, id: '', plural: false, forms: [] as string[], fuzzy: false };
  let field = '';
  const close = () => {
    if (entry.id && !entry.fuzzy && entry.forms.length && entry.forms.every((form) => form !== '')) {
      const key = entry.context === null ? entry.id : entry.context + CONTEXT_END + entry.id;
      out[key] = entry.plural ? entry.forms : entry.forms[0];
    }
    entry = { context: null, id: '', plural: false, forms: [], fuzzy: false };
    field = '';
  };
  for (const raw of [...text.split('\n'), '']) {
    let line = raw.trim();
    if (!line) {
      close();
      continue;
    }
    if (line.startsWith('#')) {
      if (line.startsWith('#,') && line.includes('fuzzy')) entry.fuzzy = true;
      continue;
    }
    if (!line.startsWith('"')) {
      const space = line.indexOf(' ');
      field = line.slice(0, space);
      line = line.slice(space + 1).trim();
    }
    const piece = unquote(line);
    if (field === 'msgctxt') entry.context = (entry.context ?? '') + piece;
    else if (field === 'msgid') entry.id += piece;
    else if (field === 'msgid_plural') entry.plural = true;
    else if (field.startsWith('msgstr')) {
      const index = field.includes('[') ? Number(field.slice(7, -1)) : 0;
      while (entry.forms.length <= index) entry.forms.push('');
      entry.forms[index] += piece;
    }
  }
  return out;
}

export function catalogs(): Plugin {
  return {
    name: 'lounge-catalogs',
    transform(code, id) {
      if (!id.endsWith('.po')) return null;
      return { code: `export default ${JSON.stringify(readCatalog(code))};`, map: null };
    },
  };
}
