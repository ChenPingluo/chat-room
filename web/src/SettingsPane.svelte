<script lang="ts">
  import { api, ApiError, formatTime, type Invite, type Me, type SessionInfo } from './lib/api';

  let {
    me,
    onError,
    onLogout,
  }: {
    me: Me;
    onError: (message: string) => void;
    onLogout: () => void;
  } = $props();

  let sessions = $state<SessionInfo[]>([]);
  let invites = $state<Invite[]>([]);
  let freshLink = $state('');
  let currentPassword = $state('');
  let newPassword = $state('');
  let copied = $state(false);

  async function load() {
    sessions = await api<SessionInfo[]>('/api/sessions');
    if (me.is_admin) invites = await api<Invite[]>('/api/invites');
  }

  $effect(() => {
    me.id;
    load().catch((err) => onError(err instanceof ApiError ? err.message : '加载失败'));
  });

  async function changePassword(event: SubmitEvent) {
    event.preventDefault();
    try {
      await api('/api/me/password', {
        method: 'POST',
        body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
      });
      currentPassword = '';
      newPassword = '';
      onError('');
      alert('密码已更新，其他设备已退出。');
    } catch (err) {
      onError(err instanceof ApiError ? err.message : '修改失败');
    }
  }

  async function revoke(id: number) {
    await api(`/api/sessions/${id}`, { method: 'DELETE' });
    if (sessions.find((session) => session.id === id)?.current) onLogout();
    else await load();
  }

  async function logoutAll() {
    await api('/api/sessions/logout-all', { method: 'POST' });
    onLogout();
  }

  async function createInvite() {
    const created = await api<{ url: string }>('/api/invites', {
      method: 'POST',
      body: JSON.stringify({ days: 7 }),
    });
    freshLink = created.url;
    await load();
  }

  async function revokeInvite(id: number) {
    await api(`/api/invites/${id}/revoke`, { method: 'POST' });
    await load();
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    copied = true;
  }
</script>

<div class="pane">
  <h2>我的账号</h2>
  <p>用户名 <strong>{me.username}</strong></p>
  <p>UID <strong class="uid">{me.uid}</strong>
    <button class="ghost" type="button" onclick={() => copy(me.uid)}>{copied ? '已复制' : '复制'}</button>
  </p>
  <p class="hint">把 UID 或用户名告诉对方。搜索必须完全一致。</p>

  <h2>修改密码</h2>
  <form class="stack" onsubmit={changePassword}>
    <input type="password" bind:value={currentPassword} placeholder="当前密码" autocomplete="current-password" required />
    <input type="password" bind:value={newPassword} placeholder="新密码，至少 10 位" autocomplete="new-password" minlength="10" required />
    <button class="primary" type="submit">更新密码</button>
  </form>

  <h2>设备</h2>
  {#each sessions as session (session.id)}
    <div class="person">
      <div>
        <strong>{session.current ? '这台设备' : '其他设备'}</strong>
        <small>{session.user_agent || '未知浏览器'} · {formatTime(session.last_seen)}</small>
      </div>
      <button class="ghost" type="button" onclick={() => revoke(session.id)}>退出</button>
    </div>
  {/each}
  <button class="danger" type="button" onclick={logoutAll}>退出所有设备</button>

  {#if me.is_admin}
    <h2>邀请链接</h2>
    <p class="hint">每条链接只能使用一次，7 天后失效。链接里的邀请码放在 # 后面，不会进入服务器日志。</p>
    <button class="primary" type="button" onclick={createInvite}>生成邀请</button>
    {#if freshLink}
      <div class="invite">
        <code>{freshLink}</code>
        <button class="ghost" type="button" onclick={() => copy(freshLink)}>复制</button>
      </div>
    {/if}
    {#each invites as invite (invite.id)}
      <div class="invite">
        <div>#{invite.id} · {invite.revoked ? '已作废' : invite.used_by ? `已使用：${invite.used_by.username}` : '未使用'}</div>
        {#if !invite.used_by && !invite.revoked}
          <button class="ghost" type="button" onclick={() => revokeInvite(invite.id)}>作废</button>
        {/if}
      </div>
    {/each}
  {/if}
</div>
