<script lang="ts">
  import { conversationTitle, formatTime, retentionLabel, RETENTIONS, type Conversation, type Message } from './lib/api';

  let {
    meId,
    conversation,
    messages,
    busy,
    onBack,
    onSendText,
    onSendImage,
    onDelete,
    onRetention,
    onLeave,
    onKick,
    onLoadOlder,
  }: {
    meId: number;
    conversation: Conversation;
    messages: Message[];
    busy: boolean;
    onBack: () => void;
    onSendText: (body: string) => Promise<void>;
    onSendImage: (file: File) => Promise<void>;
    onDelete: (id: number) => Promise<void>;
    onRetention: (secs: number) => Promise<void>;
    onLeave: () => Promise<void>;
    onKick: (userId: number) => Promise<void>;
    onLoadOlder: () => Promise<void>;
  } = $props();

  let draft = $state('');
  let lightbox = $state<number | null>(null);
  let scroller = $state<HTMLDivElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  const title = $derived(conversationTitle(conversation, meId));
  const canManage = $derived(conversation.kind === 'group' && conversation.my_role === 'owner');
  const canRetention = $derived(conversation.kind === 'direct' || canManage);

  $effect(() => {
    messages.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  function nameOf(id: number): string {
    return conversation.members.find((member) => member.id === id)?.username ?? '成员';
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const body = draft;
    if (!body.trim()) return;
    draft = '';
    await onSendText(body);
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      const body = draft;
      if (!body.trim()) return;
      draft = '';
      void onSendText(body);
    }
  }

  async function chosen(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (file) await onSendImage(file);
  }
</script>

<header class="chat-head">
  <div class="row">
    <button class="ghost back" type="button" onclick={onBack}>返回</button>
    <div>
      <strong>{title}</strong>
      <div class="meta">消息在 {retentionLabel(conversation.retention_secs)} 后自动删除</div>
    </div>
  </div>
  <div class="row" style="margin-top: 10px">
    {#if canRetention}
      <select
        aria-label="保留时间"
        value={conversation.retention_secs}
        onchange={(event) => onRetention(Number((event.target as HTMLSelectElement).value))}
      >
        {#each RETENTIONS as item (item.secs)}
          <option value={item.secs}>{item.label}</option>
        {/each}
      </select>
    {/if}
    {#if conversation.kind === 'group'}
      <button class="ghost" type="button" onclick={() => onLeave()}>退出群</button>
    {/if}
  </div>
</header>

<div class="messages" bind:this={scroller}>
  {#if messages.length >= 50}
    <button class="ghost" type="button" onclick={() => onLoadOlder()}>更早的消息</button>
  {/if}
  {#each messages as message (message.id)}
    <article class="bubble" class:mine={message.sender_id === meId}>
      {#if conversation.kind === 'group' && !message.deleted}
        <div class="who">{nameOf(message.sender_id)}</div>
      {/if}
      {#if message.deleted}
        <p class="meta">消息已删除</p>
      {:else if message.kind === 'image' && message.attachment}
        <button class="pic" type="button" onclick={() => (lightbox = message.attachment?.id ?? null)}>
          <img src={`/api/attachments/${message.attachment.id}/thumb`} alt="图片" />
        </button>
        {#if message.body}<p>{message.body}</p>{/if}
      {:else}
        <p>{message.body}</p>
      {/if}
      <span class="time">
        {formatTime(message.created_at)}
        {#if message.sender_id === meId && !message.deleted}
          <button class="ghost" type="button" onclick={() => onDelete(message.id)}>删除</button>
        {/if}
      </span>
    </article>
  {/each}
</div>

{#if canManage}
  <div class="pane-head">
    {#each conversation.members.filter((member) => member.id !== meId) as member (member.id)}
      <button class="ghost" type="button" onclick={() => onKick(member.id)}>移出 {member.username}</button>
    {/each}
  </div>
{/if}

<form class="composer" onsubmit={submit}>
  <input bind:this={fileInput} type="file" accept="image/jpeg,image/png,image/webp,image/gif" hidden onchange={chosen} />
  <button class="ghost" type="button" onclick={() => fileInput?.click()} disabled={busy}>图片</button>
  <textarea bind:value={draft} rows="2" maxlength="4000" placeholder="写一条消息" onkeydown={onKey}></textarea>
  <button class="primary" type="submit" disabled={busy}>发送</button>
</form>

{#if lightbox}
  <button class="lightbox" type="button" onclick={() => (lightbox = null)} aria-label="关闭图片">
    <img src={`/api/attachments/${lightbox}`} alt="图片" />
  </button>
{/if}
