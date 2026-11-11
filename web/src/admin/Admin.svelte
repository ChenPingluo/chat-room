<script>
  import { ApiError, api, fmt, retentionLabel } from "../lib/api.js";
  import ThemeSwitch from "../lib/ThemeSwitch.svelte";

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
  let copied = $state(false);
  const tabs = [
    ["status", "状态"],
    ["invites", "邀请"],
    ["users", "用户"],
    ["conversations", "会话"],
    ["audit", "日志"],
  ];

  function showError(err) {
    error = err instanceof ApiError ? err.message : "网络异常";
  }

  async function load() {
    try {
      if (tab === "status") status = await api("/api/status");
      if (tab === "users") users = (await api("/api/users")).users;
      if (tab === "invites") invites = (await api("/api/invites")).invites;
      if (tab === "conversations") conversations = (await api("/api/conversations")).conversations;
      if (tab === "audit") audit = (await api("/api/audit")).audit;
    } catch (err) {
      showError(err);
    }
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
    try {
      await api("/api/logout", { method: "POST" });
      me = null;
    } catch (err) {
      showError(err);
    }
  }

  async function select(next) {
    tab = next;
    error = "";
    await load();
  }

  async function createInvite(event) {
    event.preventDefault();
    error = "";
    copied = false;
    try {
      created = await api("/api/invites", { method: "POST", body: { days: Number(days) } });
      await load();
    } catch (err) {
      showError(err);
    }
  }

  async function copyInvite() {
    try {
      await navigator.clipboard.writeText(created.url);
      copied = true;
    } catch {
      error = "复制失败";
    }
  }

  async function revoke(id) {
    error = "";
    try {
      await api(`/api/invites/${id}/revoke`, { method: "POST" });
      await load();
    } catch (err) {
      showError(err);
    }
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
    error = "";
    try {
      await api(`/api/users/${id}/password`, { method: "POST", body: { password } });
      password = "";
    } catch (err) {
      showError(err);
    }
  }

  async function remove(id) {
    if (!confirm("删除用户会清除其私聊和消息，不能恢复。")) return;
    error = "";
    try {
      await api(`/api/users/${id}`, { method: "DELETE" });
      await load();
    } catch (err) {
      showError(err);
    }
  }

  async function vacuum() {
    error = "";
    try {
      await api("/api/vacuum", { method: "POST" });
      await load();
    } catch (err) {
      showError(err);
    }
  }

  function mb(bytes) {
    return `${(bytes / 1048576).toFixed(1)} MB`;
  }

  function uptime(secs) {
    const total = Math.floor(secs / 60);
    const hours = Math.floor(total / 60);
    const minutes = total % 60;
    if (hours <= 0) return `${total} 分钟`;
    return `${hours} 小时 ${minutes} 分钟`;
  }

  boot();
</script>

{#if !me}
  <main class="gate">
    <form class="card glass" onsubmit={login}>
      <div class="card-top">
        <div class="brand">
          <img class="logo" src="/icon-192.png" alt="" />
          <div>
            <h1>管理后台</h1>
            <p>只监听本机。管理员必须开启两步验证。</p>
          </div>
        </div>
        <ThemeSwitch compact />
      </div>
      <label class="field">用户名 <input bind:value={form.username} placeholder="用户名" autocomplete="username" required /></label>
      <label class="field">密码 <input type="password" bind:value={form.password} placeholder="密码" autocomplete="current-password" required /></label>
      <label class="field">验证码 <input bind:value={form.totp} placeholder="验证码" inputmode="numeric" autocomplete="one-time-code" required /></label>
      <p class="error" role="alert">{error}</p>
      <button class="submit">登录</button>
    </form>
  </main>
{:else}
  <div class="shell admin">
    <header class="topbar glass">
      <img class="logo" src="/icon-192.png" width="36" height="36" alt="" />
      <strong>管理</strong>
      <nav class="tabs" aria-label="管理分区">
        {#each tabs as [id, label] (id)}
          <button type="button" class="tab" class:on={tab === id} aria-current={tab === id ? "page" : undefined} onclick={() => select(id)}>{label}</button>
        {/each}
      </nav>
      <span class="grow"></span>
      <ThemeSwitch compact />
      <button type="button" class="ghost" onclick={logout}>退出</button>
    </header>
    <main class="canvas">
      <div class="board">
        <p class="error" role="alert">{error}</p>
        {#if tab === "status" && status}
          <div class="stats">
            <div class="stat"><span class="muted">内存</span><b>{mb(status.rss_bytes)}</b></div>
            <div class="stat"><span class="muted">用户</span><b>{status.users}</b></div>
            <div class="stat"><span class="muted">连接</span><b>{status.ws_connections}</b></div>
            <div class="stat"><span class="muted">会话</span><b>{status.conversations}</b></div>
            <div class="stat"><span class="muted">消息</span><b>{status.messages}</b></div>
            <div class="stat"><span class="muted">磁盘</span><b>{mb(status.disk_bytes)}</b></div>
          </div>
          <section class="panel">
            <p class="muted">已运行 {uptime(status.uptime_secs)}。这里不显示聊天内容。图片占用 {mb(status.image_bytes)}。</p>
            <button type="button" onclick={vacuum}>压缩数据库</button>
          </section>
        {:else if tab === "invites"}
          <section class="panel">
            <h2>生成邀请</h2>
            <form class="row" onsubmit={createInvite}>
              <input class="narrow" type="number" min="1" max="30" bind:value={days} aria-label="有效天数" />
              <button>生成 {days} 天邀请</button>
            </form>
            {#if created}
              <p>链接只显示一次：</p>
              <code>{created.url}</code>
              <button type="button" class="ghost" onclick={copyInvite}>{copied ? "已复制" : "复制"}</button>
            {/if}
          </section>
          <div class="table-wrap">
            <table>
              <thead><tr><th>时间</th><th>到期</th><th>状态</th><th></th></tr></thead>
              <tbody>
                {#each invites as invite (invite.id)}
                  <tr>
                    <td>{fmt(invite.created_at)}</td>
                    <td>{fmt(invite.expires_at)}</td>
                    <td>{#if invite.used_by}<span class="tag quiet">已用 {invite.used_by}</span>{:else if invite.revoked}<span class="tag bad">已作废</span>{:else}<span class="tag ok">可用</span>{/if}</td>
                    <td>{#if !invite.used_by && !invite.revoked}<button type="button" class="danger" onclick={() => revoke(invite.id)}>作废</button>{/if}</td>
                  </tr>
                {:else}
                  <tr><td colspan="4" class="muted">还没有邀请。</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else if tab === "users"}
          <section class="panel">
            <h2>重置密码</h2>
            <input bind:value={password} placeholder="重置时使用的新密码" aria-label="重置时使用的新密码" />
          </section>
          <div class="table-wrap">
            <table>
              <thead><tr><th>用户</th><th>状态</th><th></th></tr></thead>
              <tbody>
                {#each users as user (user.id)}
                  <tr>
                    <td>{user.display_name}<br /><small class="muted">@{user.username} · {user.uid}</small></td>
                    <td class="tags">
                      <span class="tag" class:bad={user.disabled} class:ok={!user.disabled}>{user.disabled ? "停用" : "正常"}</span>
                      {#if user.is_admin}<span class="tag">管理员</span>{/if}
                      {#if user.totp_enabled}<span class="tag quiet">两步验证</span>{/if}
                    </td>
                    <td class="actions">
                      {#if user.disabled}<button type="button" onclick={() => act(`/api/users/${user.id}/enable`)}>启用</button>{:else}<button type="button" class="ghost" onclick={() => act(`/api/users/${user.id}/disable`)}>停用</button>{/if}
                      <button type="button" class="ghost" onclick={() => reset(user.id)}>重置密码</button>
                      {#if user.is_admin}<button type="button" class="ghost" onclick={() => act(`/api/users/${user.id}/admin/revoke`)}>取消管理</button>{:else}<button type="button" class="ghost" onclick={() => act(`/api/users/${user.id}/admin`)}>设为管理</button>{/if}
                      <button type="button" class="danger" onclick={() => remove(user.id)}>删除</button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else if tab === "conversations"}
          <div class="table-wrap">
            <table>
              <thead><tr><th>编号</th><th>类型</th><th>人数</th><th>消息</th><th>保留</th><th>图片</th></tr></thead>
              <tbody>
                {#each conversations as item (item.id)}
                  <tr>
                    <td>{item.id}</td>
                    <td>{item.kind === "group" ? "群聊" : "私聊"}</td>
                    <td>{item.members}</td>
                    <td>{item.messages}</td>
                    <td>{retentionLabel(item.retention)}</td>
                    <td>{mb(item.image_bytes)}</td>
                  </tr>
                {:else}
                  <tr><td colspan="6" class="muted">还没有会话。</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <div class="table-wrap">
            <table>
              <thead><tr><th>时间</th><th>操作</th><th>对象</th></tr></thead>
              <tbody>
                {#each audit as item, index (`${item.created_at}-${index}`)}
                  <tr><td>{fmt(item.created_at)}</td><td>{item.action}</td><td>{item.detail}</td></tr>
                {:else}
                  <tr><td colspan="3" class="muted">还没有日志。</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
    </main>
  </div>
{/if}
