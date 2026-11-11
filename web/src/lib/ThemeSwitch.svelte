<script>
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import { getTheme, setTheme, subscribe } from "./theme.js";

  let { compact = false } = $props();
  let theme = $state(getTheme());
  const choices = [
    ["auto", "跟随系统", "auto"],
    ["light", "浅色", "sun"],
    ["dark", "深色", "moon"],
  ];

  function choose(next) {
    setTheme(next);
  }

  onMount(() => subscribe((value) => {
    theme = value;
  }));
</script>

<div class="themes" class:compact role="group" aria-label="外观">
  {#each choices as [id, label, icon] (id)}
    <button type="button" class:on={theme === id} aria-pressed={theme === id} title={label} onclick={() => choose(id)}>
      <Icon name={icon} size={18} />
      {#if !compact}<span>{label}</span>{/if}
    </button>
  {/each}
</div>
