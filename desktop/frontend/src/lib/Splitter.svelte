<script lang="ts">
  export let label: string;
  export let controls: string;
  export let value: number;
  export let min: number;
  export let max: number;
  export let direction = 1;
  export let onresize: (value: number) => void;
  export let oncommit: () => void;
  let drag: { pointer: number; x: number; value: number } | null = null;

  function resize(next: number) {
    onresize(Math.round(Math.max(min, Math.min(max, next))));
  }
  function start(event: PointerEvent) {
    if (event.button !== 0 || drag) return;
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    handle.focus();
    drag = { pointer: event.pointerId, x: event.clientX, value };
    event.preventDefault();
  }
  function move(event: PointerEvent) {
    if (drag?.pointer === event.pointerId)
      resize(drag.value + (event.clientX - drag.x) * direction);
  }
  function finish(event: PointerEvent) {
    if (drag?.pointer !== event.pointerId) return;
    drag = null;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId))
      handle.releasePointerCapture(event.pointerId);
    oncommit();
  }
  function keydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 40 : 10;
    let next: number;
    if (event.key === 'ArrowLeft') next = value - step * direction;
    else if (event.key === 'ArrowRight') next = value + step * direction;
    else if (event.key === 'Home') next = min;
    else if (event.key === 'End') next = max;
    else return;
    event.preventDefault();
    event.stopPropagation();
    resize(next);
    oncommit();
  }
</script>

<!-- A focusable separator is an ARIA widget, though Svelte classifies the role as static.
     https://www.w3.org/WAI/ARIA/apg/patterns/windowsplitter/ -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions -->
<div
  class="splitter"
  class:dragging={!!drag}
  role="separator"
  tabindex="0"
  aria-label={label}
  aria-controls={controls}
  aria-orientation="vertical"
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={value}
  aria-valuetext={`${value} pixels`}
  title={`${label} · arraste ou use ← / →`}
  onpointerdown={start}
  onpointermove={move}
  onpointerup={finish}
  onpointercancel={finish}
  onlostpointercapture={finish}
  onkeydown={keydown}
></div>
