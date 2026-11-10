<script lang="ts">
  import {
    api,
    ApiError,
    connectSocket,
    conversationTitle,
    type Conversation,
    type FriendRequest,
    type Me,
    type Message,
    type PublicUser,
    type ServerEvent,
  } from './lib/api';
  import ChatPane from './ChatPane.svelte';
  import FriendsPane from './FriendsPane.svelte';
  import Login from './Login.svelte';
  import Register from './Register.svelte';
  import SettingsPane from './SettingsPane.svelte';

  let me = $state<Me | null>(null);
  let loading = $state(true);
  let error = $state('');
  let view = $state<'chat' | 'friends' | 'settings'>('chat');
  let conversations = $state<Conversation[]>([]);
  let activeId = $state<number | null>(null);
  let messages = $state<Message[]>([]);
  let friends = $state<PublicUser[]>([]);
  let incoming = $state<FriendRequest[]>([]);
  let outgoing = $state<FriendRequest[]>([]);
  let busy = $state(false);
  let showGroup = $state(false);
  let groupName = $state('');
  let picked = $state<number[]>([]);

  const token = inviteToken();
  const active = $derived(conversations.find((conversation) => conversation.id === activeId) ?? null);

  function inviteToken(): string {
    const hash = decodeURIComponent(location.hash.replace(/^#/, ''));
    if (location.pathname === '/invite' || location.pathname === '/invite/') return hash;
    const parts = location.pathname.split('/').filter(Boolean);
    if (parts[0] === 'invite' && parts[1]) return decodeURIComponent(parts[1]);
    return '';
  }

  async function boot() {
    try {
      me = await api<Me>('/api/me');
      await refreshAll();
    } catch (err) {
      if (!(err instanceof ApiError) || err.status !== 401) {
        error = err instanceof Error ? err.message : '加载失败';
      }
      me = null;
    } finally {
      loading = false;
    }
  }

  async function refreshAll() {
    const [list, people, requests] = await Promise.all([
      api<Conversation[]>('/api/conversations'),
      api<PublicUser[]>('/api/friends'),
      api<{ incoming: FriendRequest[]; outgoing: FriendRequest[] }>('/api/friends/requests'),
    ]);
    conversations = list;
    friends = people;
    incoming = requests.incoming;
    outgoing = requests.outgoing;
  }

  async function openConversation(id: number) {
    activeId = id;
    view = 'chat';
    const page = await api<{ messages: Message[] }>(`/api/conversations/${id}/messages?limit=50`);
    messages = page.messages;
    const last = messages.at(-1);
    if (last) {
      await api(`/api/conversations/${id}/read`, {
        method: 'POST',
        body: JSON.stringify({ message_id: last.id }),
      });
      await refreshAll();
    }
  }

  function onEvent(event: ServerEvent) {
    if (event.type === 'message') {
      if (event.message.conv_id === activeId && !messages.some((message) => message.id === event.message.id)) {
        messages = [...messages, event.message];
      }
      void refreshAll();
    } else if (event.type === 'message_deleted') {
      messages = messages.map((message) =>
        message.id === event.message_id ? { ...message, deleted: true, body: null, attachment: null } : message,
      );
      void refreshAll();
    } else if (event.type === 'conversation_removed') {
      if (activeId === event.conv_id) activeId = null;
      void refreshAll();
    } else if (event.type === 'conversation' || event.type === 'friend_request' || event.type === 'friendship') {
      void refreshAll();
    } else if (event.type === 'error' && event.error) {
      error = event.error;
    }
  }

  $effect(() => {
    if (!me) return;
    return connectSocket(onEvent);
  });

  $effect(() => {
    void boot();
  });

  async function sendText(body: string) {
    if (!activeId) return;
    busy = true;
    try {
      const message = await api<Message>(`/api/conversations/${activeId}/messages`, {
        method: 'POST',
        body: JSON.stringify({ kind: 'text', body }),
      });
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refreshAll();
    } catch (err) {
      error = err instanceof ApiError ? err.message : '发送失败';
    } finally {
      busy = false;
    }
  }

  async function sendImage(file: File) {
    if (!activeId) return;
    if (file.size > 10 * 1024 * 1024) {
      error = '图片不能超过 10MB';
      return;
    }
    busy = true;
    try {
      const form = new FormData();
      form.append('file', file);
      const attachment = await api<{ id: number }>('/api/uploads', { method: 'POST', body: form });
      const message = await api<Message>(`/api/conversations/${activeId}/messages`, {
        method: 'POST',
        body: JSON.stringify({ kind: 'image', attachment_id: attachment.id }),
      });
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refreshAll();
    } catch (err) {
      error = err instanceof ApiError ? err.message : '上传失败';
    } finally {
      busy = false;
    }
  }

  async function removeMessage(id: number) {
    if (!activeId) return;
    await api(`/api/conversations/${activeId}/messages/${id}`, { method: 'DELETE' });
    messages = messages.map((message) =>
      message.id === id ? { ...message, deleted: true, body: null, attachment: null } : message,
    );
  }

  async function setRetention(secs: number) {
    if (!activeId) return;
    await api(`/api/conversations/${activeId}`, {
      method: 'PATCH',
      body: JSON.stringify({ retention_secs: secs }),
    });
    await refreshAll();
  }

  async function leave() {
    if (!activeId) return;
    await api(`/api/conversations/${activeId}/leave`, { method: 'POST' });
    activeId = null;
    await refreshAll();
  }

  async function kick(userId: number) {
    if (!activeId) return;
    await api(`/api/conversations/${activeId}/members/${userId}`, { method: 'DELETE' });
    await refreshAll();
  }

  async function loadOlder() {
    if (!activeId || messages.length === 0) return;
    const page = await api<{ messages: Message[] }>(
      `/api/conversations/${activeId}/messages?before=${messages[0].id}&limit=50`,
    );
    const known = new Set(messages.map((message) => message.id));
    messages = [...page.messages.filter((message) => !known.has(message.id)), ...messages];
  }

  async function startDirect(userId: number) {
    try {
      const conversation = await api<Conversation>('/api/conversations', {
        method: 'POST',
        body: JSON.stringify({ kind: 'direct', user_id: userId }),
      });
      await refreshAll();
      await openConversation(conversation.id);
    } catch (err) {
      error = err instanceof ApiError ? err.message : '无法开始私聊';
    }
  }

  async function createGroup(event: SubmitEvent) {
    event.preventDefault();
    try {
      const conversation = await api<Conversation>('/api/conversations', {
        method: 'POST',
        body: JSON.stringify({ kind: 'group', name: groupName, member_ids: picked }),
      });
      showGroup = false;
      groupName = '';
      picked = [];
      await refreshAll();
      await openConversation(conversation.id);
    } catch (err) {
      error = err instanceof ApiError ? err.message : '建群失败';
    }
  }

  function toggle(id: number) {
    picked = picked.includes(id) ? picked.filter((item) => item !== id) : [...picked, id];
  }

  async function logout() {
    await api('/api/auth/logout', { method: 'POST' });
    me = null;
    activeId = null;
  }
</script>

{#if loading}
  <main class="gate"><p>正在打开……</p></main>
{:else if !me && token}
  <Register {token} />
{:else if !me}
  <Login />
{:else}
  <div class="app" class:chatting={activeId !== null && view === 'chat'}>
    <aside class="sidebar">
      <div class="side-head">
        <div class="brand">
          <img src="/icon.svg" alt="" width="32" height="32" />
          <strong>私语</strong>
        </div>
        <button class="ghost" type="button" onclick={() => (showGroup = true)} disabled={friends.length === 0}>建群</button>
      </div>
      <nav class="nav">
        <button type="button" class:active={view === 'chat'} onclick={() => (view = 'chat')}>会话</button>
        <button type="button" class:active={view === 'friends'} onclick={() => (view = 'friends')}>
          好友 {#if incoming.length}<span class="badge">{incoming.length}</span>{/if}
        </button>
        <button type="button" class:active={view === 'settings'} onclick={() => (view = 'settings')}>设置</button>
      </nav>
      {#if error}<div class="error">{error}</div>{/if}
      {#if view === 'chat'}
        <div class="list">
          {#if conversations.length === 0}
            <p class="hint">还没有会话。去好友里搜索对方的完整用户名或 UID。</p>
          {/if}
          {#each conversations as conversation (conversation.id)}
            <button class="conv" class:active={conversation.id === activeId} type="button" onclick={() => openConversation(conversation.id)}>
              <span>
                <strong>{conversationTitle(conversation, me.id)}</strong>
                <small>{conversation.last_message?.deleted ? '消息已删除' : conversation.last_message?.body || (conversation.last_message?.kind === 'image' ? '[图片]' : '还没有消息')}</small>
              </span>
              {#if conversation.unread > 0}<span class="badge">{conversation.unread}</span>{/if}
            </button>
          {/each}
        </div>
      {:else if view === 'friends'}
        <FriendsPane
          {friends}
          {incoming}
          {outgoing}
          onChanged={refreshAll}
          onError={(message) => (error = message)}
        />
        <div class="pane">
          {#each friends as friend (friend.id)}
            <button class="ghost" type="button" onclick={() => startDirect(friend.id)}>和 {friend.username} 私聊</button>
          {/each}
        </div>
      {:else}
        <SettingsPane {me} onError={(message) => (error = message)} onLogout={logout} />
        <div class="pane"><button class="ghost" type="button" onclick={logout}>退出登录</button></div>
      {/if}
    </aside>
    <section class="main">
      {#if active && view === 'chat'}
        <ChatPane
          meId={me.id}
          conversation={active}
          {messages}
          {busy}
          onBack={() => (activeId = null)}
          onSendText={sendText}
          onSendImage={sendImage}
          onDelete={removeMessage}
          onRetention={setRetention}
          onLeave={leave}
          onKick={kick}
          onLoadOlder={loadOlder}
        />
      {:else}
        <div class="empty">
          <h2>这里只有你认识的人</h2>
          <p>搜索好友的完整用户名或 7 位 UID，对方同意后才能私聊或拉群。消息默认 30 天后删除。</p>
        </div>
      {/if}
    </section>
  </div>
{/if}

{#if showGroup && me}
  <div class="modal-back">
    <form class="card" onsubmit={createGroup}>
      <h2>新建群</h2>
      <label for="group-name">群名</label>
      <input id="group-name" bind:value={groupName} maxlength="40" required />
      {#each friends as friend (friend.id)}
        <label class="check">
          <input type="checkbox" checked={picked.includes(friend.id)} onchange={() => toggle(friend.id)} />
          {friend.username}
        </label>
      {/each}
      <div class="row">
        <button class="ghost" type="button" onclick={() => (showGroup = false)}>取消</button>
        <button class="primary" type="submit">创建</button>
      </div>
    </form>
  </div>
{/if}
