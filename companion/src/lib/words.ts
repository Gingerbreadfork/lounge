// What each pin does, in the same words the TV's menu uses, so both screens explain themselves alike.
export const words = {
  pinApp: 'Pin to the front of Your apps',
  pinAppDetail: 'Stays put ahead of recent apps',
  unpinApp: 'Unpin from Your apps',
  unpinAppDetail: 'Follows what you open again',
  pinStage: 'Pin to the Stage',
  pinStageDetail: 'Stays in the big pictures up top',
  unpinStage: 'Unpin from the Stage',
  unpinStageDetail: 'What you watch takes its place',
  hide: 'Hide',
  hideDetail: 'Off every row until you show it',
  hideAllFromDetail: 'Off Continue watching until you show it',
  hideRow: 'Hide this row',
  hideRowDetail: 'The Layout tab brings it back',
};

/** A count with its noun: "1 minute", "15 minutes". */
export function counted(n: number, one: string) {
  return `${n} ${one}${n === 1 ? '' : 's'}`;
}

/** A length of time in words: "under a minute", "5 minutes", "2 hours 5 minutes", "3 days". */
export function span(ms: number): string {
  const m = Math.floor(ms / 60000);
  if (m < 1) return 'under a minute';
  if (m < 60) return counted(m, 'minute');
  const h = Math.floor(m / 60);
  if (h < 48) return m % 60 ? `${counted(h, 'hour')} ${counted(m % 60, 'minute')}` : counted(h, 'hour');
  return counted(Math.floor(h / 24), 'day');
}

/** How long ago `ms` was, in words: "just now", "3 minutes ago", "2 hours ago", "4 days ago". */
export function ago(ms: number): string {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return 'just now';
  if (m < 60) return `${counted(m, 'minute')} ago`;
  if (m < 1440) return `${counted(Math.floor(m / 60), 'hour')} ago`;
  return `${counted(Math.floor(m / 1440), 'day')} ago`;
}
