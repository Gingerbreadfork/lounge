<script lang="ts">
  // A button that asks before it destroys, as the TV does: the first press turns it into the
  // question in ember, the second does it. Left alone, or left for something else, it goes back.
  let {
    label,
    confirm,
    onconfirm,
    quiet = false,
    disabled = false,
  }: { label: string; confirm: string; onconfirm: () => void; quiet?: boolean; disabled?: boolean } = $props();

  let armed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function press() {
    clearTimeout(timer);
    if (!armed) {
      armed = true;
      timer = setTimeout(() => (armed = false), 6000);
      return;
    }
    armed = false;
    onconfirm();
  }
  function disarm() {
    clearTimeout(timer);
    armed = false;
  }

  $effect(() => () => clearTimeout(timer));
</script>

<button class="btn" class:ghost={quiet && !armed} class:danger={!armed} class:armed {disabled} onclick={press} onblur={disarm}>{armed ? confirm : label}</button>

<style>
  .armed { background: var(--ember); color: var(--velvet); box-shadow: none; }
</style>
