<script>
  import { ApiError, api, fmt, retentionLabel } from "../lib/api.js";

  let me = $state(null);
  let error = $state("");
  let tab = $state("status");
  let form = $state({ username: "", password: "", totp: "" });
  let status = $state(null);
  let users = $state([]);
  let invites = $state([]);
  let conversations = $state([]);
  let audit = $state([]);
  let created = $state(null);
  let days = $state(7);
  let password = $state("");

  function showError(err) {
    error = err instanceof ApiError ? err.message : "网络异常";
  }

  async function load() {
    if (tab === "status") status = await api("/api/status");
    if (tab === "users") users = (await api("/api/users")).users;
    if (tab === "invites") invites = (await api("/api/invites")).invites;
    if (tab === "conversations") conversations = (await api("/api/conversations")).conversations;
    if (tab === "audit") audit = (await api("/api/audit")).audit;
  }

  async function boot() {
    try {
      me = await api("/api/me");
    } catch {
      me = null;
    }
    if (me) await load();
  }

  async function login(event) {
    event.preventDefault();
    error = "";
    try {
      me = await api("/api/login", { method: "POST", body: form });
      await load();
    } catch (err) {
      showError(err);
    }
  }

  async function logout() {
    await api("/api/logout", { method: "POST" });
    me = null;
  }

  async function select(next) {
    tab = next;
    error = "";
    await load();
  }

  async function createInvite(event) {
    event.preventDefault();
    created = await api("/api/invites", { method: "POST", body: { days: Number(days) } });
    await load();
  }

  async function revoke(id) {
    await api(`/api/invites/${id}/revoke`, { method: "POST" });
    await load();
  }

  async function act(path) {
    error = "";
    try {
      await api(path, { method: "POST" });
      await load();
    } catch (err) {
      showError(err);
    }
  }

  async function reset(id) {
    await api(`/api/users/${id}/password`, { method: "POST", body: { password } });
    password = "";
    error = "";
  }

  async function remove(id) {
    if (!confirm("删除用户会清除其私聊和消息，不能恢复。")) return;
    await api(`/api/users/${id}`, { method: "DELETE" });
    await load();
  }

  async function vacuum() {
    await api("/api/vacuum", { method: "POST" });
    await load();
  }

  function mb(bytes) {
    return `${(bytes / 1048576).toFixed(1)} MB`;
  }

  boot();
</script>

{#if !me}
  <form class="card" onsubmit={login}>
    <h1>管理后台</h1>
    <p class="muted">只监听本机。管理员必须开启两步验证。</p>
    <input bind:value={form.username} placeholder="用户名" autocomplete="username" required />
    <input type="password" bind:value={form.password} placeholder="密码" autocomplete="current-password" required />
    <input bind:value={form.totp} placeholder="验证码" inputmode="numeric" autocomplete="one-time-code" required />
    <p class="error">{error}</p>
    <button>登录</button>
  </form>
{:else}
  <div class="app">
    <header class="top">
      <strong>管理</strong>
      <nav>
        <button class="tab" class:on={tab === "status"} onclick={() => select("status")}>状态</button>
        <button class="tab" class:on={tab === "invites"} onclick={() => select("invites")}>邀请</button>
        <button class="tab" class:on={tab === "users"} onclick={() => select("users")}>用户</button>
        <button class="tab" class:on={tab === "conversations"} onclick={() => select("conversations")}>会话</button>
        <button class="tab" class:on={tab === "audit"} onclick={() => select("audit")}>日志</button>
      </nav>
      <span class="grow"></span>
      <button class="ghost" onclick={logout}>退出</button>
    </header>
    <section class="page">
      <p class="error">{error}</p>
      {#if tab === "status" && status}
        <div class="stats">
          <div class="stat"><b>{mb(status.rss_bytes)}</b><br /><span class="muted">内存</span></div>
          <div class="stat"><b>{status.users}</b><br /><span class="muted">用户</span></div>
          <div class="stat"><b>{status.ws_connections}</b><br /><span class="muted">连接</span></div>
          <div class="stat"><b>{status.messages}</b><br /><span class="muted">消息</span></div>
          <div class="stat"><b>{mb(status.disk_bytes)}</b><br /><span class="muted">磁盘</span></div>
        </div>
        <p class="muted">已运行 {Math.floor(status.uptime_secs / 60)} 分钟。这里不显示聊天内容。</p>
        <button onclick={vacuum}>压缩数据库</button>
      {:else if tab === "invites"}
        <form class="row" onsubmit={createInvite}>
          <input type="number" min="1" max="30" bind:value={days} />
          <button>生成 {days} 天邀请</button>
        </form>
        {#if created}
          <p>链接只显示一次：<code>{created.url}</code></p>
          <button class="ghost" onclick={() => navigator.clipboard.writeText(created.url)}>复制</button>
        {/if}
        <table>
          <thead><tr><th>时间</th><th>到期</th><th>状态</th><th></th></tr></thead>
          <tbody>
            {#each invites as invite}
              <tr>
                <td>{fmt(invite.created_at)}</td>
                <td>{fmt(invite.expires_at)}</td>
                <td>{invite.used_by ? `已用 ${invite.used_by}` : invite.revoked ? "已作废" : "可用"}</td>
                <td>{#if !invite.used_by && !invite.revoked}<button class="danger" onclick={() => revoke(invite.id)}>作废</button>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if tab === "users"}
        <input bind:value={password} placeholder="重置时使用的新密码" />
        <table>
          <thead><tr><th>用户</th><th>状态</th><th></th></tr></thead>
          <tbody>
            {#each users as user}
              <tr>
                <td>{user.display_name}<br /><small class="muted">@{user.username} · {user.uid}</small></td>
                <td>{user.disabled ? "停用" : "正常"}{user.is_admin ? " · 管理员" : ""}{user.totp_enabled ? " · 两步验证" : ""}</td>
                <td class="row">
                  {#if user.disabled}<button onclick={() => act(`/api/users/${user.id}/enable`)}>启用</button>{:else}<button class="ghost" onclick={() => act(`/api/users/${user.id}/disable`)}>停用</button>{/if}
                  <button class="ghost" onclick={() => reset(user.id)}>重置密码</button>
                  {#if user.is_admin}<button class="ghost" onclick={() => act(`/api/users/${user.id}/admin/revoke`)}>取消管理</button>{:else}<button class="ghost" onclick={() => act(`/api/users/${user.id}/admin`)}>设为管理</button>{/if}
                  <button class="danger" onclick={() => remove(user.id)}>删除</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if tab === "conversations"}
        <table>
          <thead><tr><th>编号</th><th>类型</th><th>人数</th><th>消息</th><th>保留</th><th>图片</th></tr></thead>
          <tbody>
            {#each conversations as item}
              <tr>
                <td>{item.id}</td><td>{item.kind === "group" ? "群聊" : "私聊"}</td><td>{item.members}</td><td>{item.messages}</td><td>{retentionLabel(item.retention)}</td><td>{mb(item.image_bytes)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <table>
          <thead><tr><th>时间</th><th>操作</th><th>对象</th></tr></thead>
          <tbody>
            {#each audit as item}
              <tr><td>{fmt(item.created_at)}</td><td>{item.action}</td><td>{item.detail}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  </div>
{/if}
