// What opens over a page: a sheet, a panel inside one, a live picture. Each is one step of the
// phone's history, so Back closes whichever is on top.

type Layer = { onback: () => void };

const stack: Layer[] = [];
/** Steps back that this file made itself, which close nothing when they arrive. */
let own = 0;
/** Layers taken away and not yet stepped back over. */
let owed = 0;

/** Steps back once for every layer taken away in this turn, so no two steps race each other. */
function stepBack() {
  if (owed++ > 0) return;
  setTimeout(() => {
    const steps = owed;
    owed = 0;
    own++;
    history.go(-steps);
  }, 0);
}

window.addEventListener('popstate', () => {
  if (own > 0) {
    own--;
    return;
  }
  stack.pop()?.onback();
});

export type Opened = {
  /** Takes the layer away without Back being pressed. It does nothing once the layer has gone. */
  leave: () => void;
  /** Whether nothing is open over this layer. */
  top: () => boolean;
};

/** Opens a layer over whatever is showing. `onback` runs when Back closes it. */
export function layer(onback: () => void): Opened {
  const l = { onback };
  stack.push(l);
  history.pushState({ layer: stack.length }, '');
  return {
    leave: () => {
      const at = stack.indexOf(l);
      if (at < 0) return;
      stack.splice(at, 1);
      stepBack();
    },
    top: () => stack[stack.length - 1] === l,
  };
}
