import { t, tc, tn } from './lang.svelte';

// What each pin does, in the same words the TV's menu uses, so both screens explain themselves alike.
export const words = {
  get pinApp() { return t('Pin to the front of Your apps'); },
  // Translators: under "Pin to the front of Your apps": what the app does once pinned
  get pinAppDetail() { return t('Stays put ahead of recent apps'); },
  get unpinApp() { return t('Unpin from Your apps'); },
  // Translators: under "Unpin from Your apps": the app moves with the ones opened lately
  get unpinAppDetail() { return t('Follows what you open again'); },
  get pinStage() { return t('Pin to the Stage'); },
  // Translators: under "Pin to the Stage": where the pinned thing stays
  get pinStageDetail() { return t('Stays in the big pictures up top'); },
  get unpinStage() { return t('Unpin from the Stage'); },
  // Translators: under "Unpin from the Stage": what follows for the place it held
  get unpinStageDetail() { return t('What you watch takes its place'); },
  // Translators: a menu choice; {title} is the name of the tile or row it hides
  hide: (title: string) => t('Hide {title}', { title }),
  // Translators: under "Hide {title}": the tile is left out of every row
  get hideDetail() { return t('Off every row until you show it'); },
  // Translators: under "Hide everything from {source}": the app or plugin is left out of the Continue watching row
  get hideAllFromDetail() { return t('Off Continue watching until you show it'); },
  get hideRow() { return t('Hide this row'); },
  // Translators: under "Hide this row": the phone's Layout tab shows the row again
  get hideRowDetail() { return t('The Layout tab brings it back'); },
  // Translators: a menu choice that opens a control's own settings
  get adjust() { return t('Adjust'); },
  // Translators: under "Adjust": what the choice opens
  get adjustDetail() { return t('Its own settings, here on the phone'); },
};

/** `items` as a list in a sentence: "A", "A and B", "A, B and C". */
export function list(items: string[]): string {
  if (items.length < 2) return items[0] ?? '';
  let said = items[0];
  // Translators: two items of a list that goes on
  for (const next of items.slice(1, -1)) said = t('{first}, {next}', { first: said, next });
  // Translators: the end of a list
  return t('{first} and {last}', { first: said, last: items[items.length - 1] });
}

/** `value` with `places` decimals, with the decimal mark of the language in use. */
export function decimal(value: number, places: number): string {
  const plain = value.toFixed(places);
  // Translators: the mark between a whole number and its decimals, a comma in many languages
  const point = tc('decimal mark', '.');
  return point === '.' ? plain : plain.replace('.', point);
}

/** A text on either side of its `{place}`, for markup that sets an element there. */
export function around(text: string, place: string): [string, string] {
  const at = text.indexOf(`{${place}}`);
  return at < 0 ? [text, ''] : [text.slice(0, at), text.slice(at + place.length + 2)];
}

/** A length of time in words: "under a minute", "5 minutes", "2 hours 5 minutes", "3 days". */
export function span(ms: number): string {
  const m = Math.floor(ms / 60000);
  if (m < 1) return t('under a minute');
  if (m < 60) return tn(m, '{n} minute', '{n} minutes');
  const h = Math.floor(m / 60);
  if (h < 48) {
    if (m % 60 === 0) return tn(h, '{n} hour', '{n} hours');
    const minutes = tn(m % 60, '{n} minute', '{n} minutes');
    // Translators: a length of time; {minutes} is a count of minutes such as "5 minutes"
    return tn(h, '{n} hour {minutes}', '{n} hours {minutes}', { minutes });
  }
  return tn(Math.floor(h / 24), '{n} day', '{n} days');
}

/** How long ago `ms` was, in words: "just now", "3 minutes ago", "2 hours ago", "4 days ago". */
export function ago(ms: number): string {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return t('just now');
  if (m < 60) return tn(m, '{n} minute ago', '{n} minutes ago');
  if (m < 1440) return tn(Math.floor(m / 60), '{n} hour ago', '{n} hours ago');
  return tn(Math.floor(m / 1440), '{n} day ago', '{n} days ago');
}
