<script lang="ts">
  export let label: string;
  export let compact = false;
  export let progress = false;
</script>

<div
  class="activity"
  class:compact
  role="status"
  aria-live="polite"
  aria-atomic="true"
>
  <span class="activity-line" title={label}>
    <span class="spinner" aria-hidden="true"></span>
    <span class:sr-only={compact}>{label}</span>
  </span>
  {#if progress}
    <div class="progress" role="progressbar" aria-label={label}><i></i></div>
  {/if}
</div>

<style>
  .activity {
    color: var(--muted);
    font-size: calc(11px * var(--font-scale));
    padding: 10px 0;
  }
  .compact {
    display: inline-flex;
    padding: 0;
    vertical-align: middle;
  }
  .activity-line {
    display: flex;
    align-items: center;
    gap: 8px;
    overflow-wrap: anywhere;
  }
  .spinner {
    width: 1em;
    height: 1em;
    flex-shrink: 0;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  .progress {
    height: 3px;
    margin-top: 9px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--line);
  }
  .progress i {
    display: block;
    height: 100%;
    width: 35%;
    background: var(--accent);
    animation: slide 1.4s ease-in-out infinite;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(390%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner,
    .progress i {
      animation: none;
    }
    .progress i {
      width: 100%;
      opacity: 0.5;
    }
  }
</style>
