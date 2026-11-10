<script lang="ts">
  import { api, ApiError, type PublicUser, type FriendRequest } from './lib/api';

  let {
    friends,
    incoming,
    outgoing,
    onChanged,
    onError,
  }: {
    friends: PublicUser[];
    incoming: FriendRequest[];
    outgoing: FriendRequest[];
    onChanged: () => Promise<void>;
    onError: (message: string) => void;
  } = $props();

  let query = $state('');
  let hits = $state<PublicUser[]>([]);
  let searched = $state(false);

  async function search(event: SubmitEvent) {
    event.preventDefault();
    try {
      const result = await api<{ users: PublicUser[] }>(`/api/users/search?q=${encodeURIComponent(query.trim())}`);
      hits = result.users;
      searched = true;
    } catch (err) {
      onError(err instanceof ApiError ? err.message : '搜索失败');
    }
  }

  async function request(userId: number) {
    try {
      await api('/api/friends/requests', { method: 'POST', body: JSON.stringify({ user_id: userId }) });
      await searchExisting();
      await onChanged();
    } catch (err) {
      onError(err instanceof ApiError ? err.message : '申请失败');
    }
  }

  async function searchExisting() {
    if (!query.trim()) return;
    const result = await api<{ users: PublicUser[] }>(`/api/users/search?q=${encodeURIComponent(query.trim())}`);
    hits = result.users;
  }

  async function accept(id: number) {
    await api(`/api/friends/requests/${id}/accept`, { method: 'POST' });
    await onChanged();
  }

  async function reject(id: number) {
    await api(`/api/friends/requests/${id}/reject`, { method: 'POST' });
    await onChanged();
  }

  async function remove(id: number) {
    await api(`/api/friends/${id}`, { method: 'DELETE' });
    await onChanged();
  }
</script>

<div class="pane">
  <h2>找人</h2>
  <p class="hint">必须输入完整用户名，或者 7 位 UID。不会列出其他人。</p>
  <form class="row" onsubmit={search}>
    <input bind:value={query} placeholder="用户名或 UID" aria-label="搜索用户" />
    <button class="primary" type="submit">搜索</button>
  </form>
  {#if searched && hits.length === 0}
    <p class="hint">没有找到这个人。</p>
  {/if}
  {#each hits as user (user.id)}
    <div class="person">
      <div>
        <strong>{user.username}</strong>
        <small class="uid">{user.uid}</small>
      </div>
      {#if user.relation === 'none'}
        <button class="primary" type="button" onclick={() => request(user.id)}>加好友</button>
      {:else if user.relation === 'pending_out'}
        <span class="meta">已申请</span>
      {:else if user.relation === 'pending_in'}
        <span class="meta">对方已申请你</span>
      {:else if user.relation === 'friends'}
        <span class="meta">已是好友</span>
      {:else if user.relation === 'self'}
        <span class="meta">这是你</span>
      {/if}
    </div>
  {/each}

  <h2>收到的申请</h2>
  {#if incoming.length === 0}<p class="hint">没有新申请。</p>{/if}
  {#each incoming as request (request.id)}
    <div class="person">
      <div>
        <strong>{request.from_user.username}</strong>
        <small class="uid">{request.from_user.uid}</small>
      </div>
      <div class="row">
        <button class="primary" type="button" onclick={() => accept(request.id)}>同意</button>
        <button class="ghost" type="button" onclick={() => reject(request.id)}>拒绝</button>
      </div>
    </div>
  {/each}

  {#if outgoing.length}
    <h2>发出的申请</h2>
    {#each outgoing as request (request.id)}
      <div class="person"><strong>{request.to_user.username}</strong><span class="meta">等待对方</span></div>
    {/each}
  {/if}

  <h2>好友</h2>
  {#if friends.length === 0}<p class="hint">还没有好友。注册后这里只有你自己。</p>{/if}
  {#each friends as friend (friend.id)}
    <div class="person">
      <div>
        <strong>{friend.username}</strong>
        <small class="uid">{friend.uid}</small>
      </div>
      <button class="danger" type="button" onclick={() => remove(friend.id)}>删除</button>
    </div>
  {/each}
</div>
