// A press held still for a moment, on touch or with a mouse. The tap that would follow is
// swallowed, so holding a tile never also opens it. With no callback the element is left alone.
export function hold(node: HTMLElement, onHold: (() => void) | null) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let startX = 0;
  let startY = 0;
  let swallowClick = false;
  const cancel = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  const fire = () => {
    cancel();
    if (!onHold) return;
    swallowClick = true;
    navigator.vibrate?.(12);
    onHold();
  };
  const down = (e: PointerEvent) => {
    // A new press starts clean: a swallowed tap from the last hold must not eat this one.
    swallowClick = false;
    if (e.button !== 0 || !onHold) return;
    startX = e.clientX;
    startY = e.clientY;
    cancel();
    timer = setTimeout(fire, 450);
  };
  const move = (e: PointerEvent) => {
    if (timer && Math.hypot(e.clientX - startX, e.clientY - startY) > 8) cancel();
  };
  const click = (e: MouseEvent) => {
    if (!swallowClick) return;
    swallowClick = false;
    e.preventDefault();
    e.stopImmediatePropagation();
  };
  const context = (e: Event) => {
    // Android fires this for a long press too, sometimes before our own timer: one hold is enough.
    e.preventDefault();
    if (timer) fire();
    else if (!swallowClick) fire();
  };
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', cancel);
  node.addEventListener('pointercancel', cancel);
  node.addEventListener('pointerleave', cancel);
  node.addEventListener('click', click, true);
  node.addEventListener('contextmenu', context);
  return {
    update(next: (() => void) | null) {
      onHold = next;
    },
    destroy() {
      cancel();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', cancel);
      node.removeEventListener('pointercancel', cancel);
      node.removeEventListener('pointerleave', cancel);
      node.removeEventListener('click', click, true);
      node.removeEventListener('contextmenu', context);
    },
  };
}
