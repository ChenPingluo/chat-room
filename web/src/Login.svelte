<script lang="ts">
  import { api, ApiError } from './lib/api';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      await api('/api/auth/login', {
        method: 'POST',
        body: JSON.stringify({ username, password }),
      });
      location.assign('/');
    } catch (err) {
      error = err instanceof ApiError ? err.message : '登录失败';
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
        <h1>私语</h1>
      </div>
    </div>
    <p class="hint">只接受邀请。没有账号的话，向管理员要一条邀请链接。</p>
    {#if error}<div class="error">{error}</div>{/if}
    <label for="username">用户名</label>
    <input id="username" bind:value={username} autocomplete="username" required />
    <label for="password">密码</label>
    <input id="password" type="password" bind:value={password} autocomplete="current-password" required />
    <button class="primary" type="submit" disabled={busy}>进入</button>
  </form>
</main>
