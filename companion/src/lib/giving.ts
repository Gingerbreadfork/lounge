// Where Lounge lives and the ways to give to its maker, as the TV's About and Support pages have
// them. A test on the TV's side holds the two lists to each other, and each address to its checksum.

import { mark } from './lang.svelte';

/** A link's `name` is held as written and goes through `tr` where it is shown. */
export type Link = { name: string; href: string; shown: string };
export type Coin = { name: string; href: string; address: string };

export const aboutLinks: Link[] = [
  { name: mark('The source code'), href: 'https://github.com/Gingerbreadfork/lounge', shown: 'github.com/Gingerbreadfork/lounge' },
  { name: mark('Report a problem'), href: 'https://github.com/Gingerbreadfork/lounge/issues', shown: 'github.com/Gingerbreadfork/lounge/issues' },
  { name: mark("What's new"), href: 'https://github.com/Gingerbreadfork/lounge/releases', shown: 'github.com/Gingerbreadfork/lounge/releases' },
  { name: mark('Gingerbreadfork on X'), href: 'https://x.com/gingerbreadfork', shown: 'x.com/gingerbreadfork' },
];

export const paypal: Link = { name: 'PayPal', href: 'https://paypal.me/gingerbreadfork', shown: 'paypal.me/gingerbreadfork' };

/** Each coin's link is its address under the coin's own scheme, which a wallet on the phone opens. */
export const coins: Coin[] = [
  { name: 'Bitcoin', href: 'bitcoin:bc1qlezapstmtq6jm4cj6ahn0ylyyfnpz79l2zegjv', address: 'bc1qlezapstmtq6jm4cj6ahn0ylyyfnpz79l2zegjv' },
  { name: 'DigiByte', href: 'digibyte:dgb1qrf5m26fjs57rs4lehdm95ycz5h8py94n3hcw2h', address: 'dgb1qrf5m26fjs57rs4lehdm95ycz5h8py94n3hcw2h' },
  { name: 'Bitcoin Cash', href: 'bitcoincash:qqn2ymq3qwp7987cjlllgyp56dk3m7t89vu8u48sxk', address: 'qqn2ymq3qwp7987cjlllgyp56dk3m7t89vu8u48sxk' },
];

/** An address in groups of four, so it can be checked by eye against what a wallet shows. */
export function spaced(address: string): string {
  return address.match(/.{1,4}/g)?.join(' ') ?? address;
}

/**
 * Puts `text` on the phone's clipboard and says whether it got there. The TV serves this page
 * over plain http, where the browser's own clipboard call is not offered, so a selection copied
 * from a field out of sight stands in for it.
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (window.isSecureContext && navigator.clipboard) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // The selection below is tried next.
  }
  const field = document.createElement('textarea');
  field.value = text;
  field.setAttribute('readonly', '');
  field.style.position = 'fixed';
  field.style.opacity = '0';
  document.body.appendChild(field);
  field.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  }
  field.remove();
  return copied;
}
