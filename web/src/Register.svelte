<script lang="ts">
  import { api, ApiError } from './lib/api';

  let {
    token,
  }: {
    token: string;
  } = $props();

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let valid = $state<boolean | null>(null);

  $effect(() => {
    const current = token;
    valid = null;
    if (!current) {
      valid = false;
      return;
    }
    api<{ valid: boolean }>('/api/auth/invite/preview', {
      method: 'POST',
      body: JSON.stringify({ token: current }),
    })
      .then(() => {
        if (current === token) valid = true;
      })
      .catch(() => {
        if (current === token) valid = false;
      });
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      await api('/api/auth/register', {
        method: 'POST',
        body: JSON.stringify({ token, username, password }),
      });
      location.assign('/');
    } catch (err) {
      error = err instanceof ApiError ? err.message : '注册失败';
    } finally {
      busy = false;
    }
  }
</script>

<main class="gate">
  <form class="card" onsubmit={submit}>
    <div class="brand">
      <img src="/icon.svg" alt="" />
      <div>
        <h1>加入私语</h1>
      </div>
    </div>
    <p class="hint">注册之后，这里只有你自己。搜索对方的完整用户名或 UID，对方同意后才能聊天。</p>
    {#if error}<div class="error">{error}</div>{/if}
    {#if valid === false}
      <div class="error">邀请链接无效或已过期。</div>
    {:else if valid === null}
      <p class="hint">正在确认邀请……</p>
    {:else}
      <label for="username">用户名</label>
      <input id="username" bind:value={username} autocomplete="username" minlength="3" maxlength="20" required />
      <label for="password">密码</label>
      <input id="password" type="password" bind:value={password} autocomplete="new-password" minlength="10" required />
      <p class="hint">至少 10 个字符。用户名以字母开头，只能包含字母、数字和下划线。</p>
      <button class="primary" type="submit" disabled={busy}>创建账号</button>
    {/if}
  </form>
</main>
