<script>
  let { name = "chat", size = 22 } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "users"}
    <path d="M15.5 19.5v-1.2a3 3 0 0 0-3-3h-5a3 3 0 0 0-3 3v1.2" />
    <circle cx="10" cy="8.2" r="2.5" />
    <path d="M19.5 19.5v-1a2.7 2.7 0 0 0-2-2.6" />
    <path d="M15.6 5.8a2.4 2.4 0 0 1 0 4.5" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3" />
    <path d="M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4 18 18M18 6l-1.6 1.6M7.6 16.4 6 18" />
  {:else if name === "logout"}
    <path d="M10 5.5H6.8A1.8 1.8 0 0 0 5 7.3v9.4a1.8 1.8 0 0 0 1.8 1.8H10" />
    <path d="M10 12h9M16 8.5 19.5 12 16 15.5" />
  {:else if name === "sun"}
    <circle cx="12" cy="12" r="3.2" />
    <path d="M12 3.5v1.8M12 18.7v1.8M3.5 12h1.8M18.7 12h1.8M6 6l1.3 1.3M16.7 16.7 18 18M18 6l-1.3 1.3M7.3 16.7 6 18" />
  {:else if name === "moon"}
    <path d="M16.5 14.8A6.2 6.2 0 0 1 9.4 6 6.3 6.3 0 1 0 16.5 14.8z" />
  {:else if name === "auto"}
    <rect x="4" y="5" width="16" height="11" rx="2" />
    <path d="M8 19.5h8" />
  {:else if name === "search"}
    <circle cx="11" cy="11" r="5.5" />
    <path d="m15.5 15.5 3.5 3.5" />
  {:else if name === "image"}
    <rect x="4" y="5" width="16" height="14" rx="2" />
    <circle cx="9" cy="10" r="1.3" />
    <path d="m8 16 2.4-2.4a1.2 1.2 0 0 1 1.7 0L16 17" />
  {:else if name === "send"}
    <path d="M4.5 12 19 5.5l-4.2 13-3.3-4.6z" />
    <path d="M11.5 13.9 19 5.5" />
  {:else if name === "back"}
    <path d="M14.5 6.5 8.5 12l6 5.5" />
  {:else if name === "close"}
    <path d="M7 7l10 10M17 7 7 17" />
  {:else if name === "info"}
    <circle cx="12" cy="12" r="7.5" />
    <path d="M12 11v5M12 8h.01" />
  {:else if name === "copy"}
    <rect x="8" y="8" width="10" height="11" rx="1.8" />
    <path d="M6 15.5H5.8A1.8 1.8 0 0 1 4 13.7V6.8A1.8 1.8 0 0 1 5.8 5H13a1.8 1.8 0 0 1 1.8 1.8V8" />
  {:else if name === "bell"}
    <path d="M6.5 16.5h11l-1.1-1.6V10a4.4 4.4 0 1 0-8.8 0v4.9z" />
    <path d="M10 17.5a2 2 0 0 0 4 0" />
  {:else if name === "shield"}
    <path d="M12 3.8 18.5 6v5.2c0 3.6-2.5 6.4-6.5 8-4-1.6-6.5-4.4-6.5-8V6z" />
    <path d="m9.2 12 1.8 1.8 3.8-4" />
  {:else if name === "key"}
    <circle cx="8.2" cy="12" r="3.2" />
    <path d="M11.2 12H20l-1.6 1.8L20 15.4h-2.2" />
  {:else if name === "devices"}
    <rect x="4" y="5.5" width="16" height="10" rx="1.8" />
    <path d="M8 19h8M12 15.5V19" />
  {:else}
    <path d="M6.2 16.8 4.5 19.5v-3.4A7 7 0 1 1 12 18.2H7.2z" />
  {/if}
</svg>
