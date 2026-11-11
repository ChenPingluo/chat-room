<script>
  import { onMount } from "svelte";
  import { ApiError, RETENTION, api, fmt, retentionLabel } from "../lib/api.js";

  let me = $state(null);
  let view = $state("boot");
  let error = $state("");
  let toast = $state("");
  let conversations = $state([]);
  let active = $state(null);
  let detail = $state(null);
  let messages = $state([]);
  let friends = $state([]);
  let requests = $state([]);
  let sessions = $state([]);
  let info = $state(false);
  let preview = $state("");
  let text = $state("");
  let login = $state({ username: "", password: "", totp: "", needTotp: false });
  let register = $state({ token: "", username: "", password: "", display_name: "" });
  let search = $state("");
  let found = $state([]);
  let groupName = $state("");
  let picked = $state([]);
  let passwords = $state({ old_password: "", new_password: "" });
  let totp = $state(null);
  let totpCode = $state("");
  let scroller = $state(null);
  let socket;
  let knownIncoming = 0;
  let toastTimer;

  function route() {
    const hash = location.hash.replace(/^#/, "") || "/chats";
    if (hash.startsWith("/register/")) return { name: "register", token: decodeURIComponent(hash.slice(10)) };
    if (hash.startsWith("/chat/")) return { name: "chat", id: Number(hash.slice(6)) };
    if (hash === "/friends") return { name: "friends" };
    if (hash === "/settings") return { name: "settings" };
    if (hash === "/login") return { name: "login" };
    return { name: "chats" };
  }

  function go(hash) {
    location.hash = hash;
  }

  function showError(err) {
    if (err instanceof ApiError && err.status === 401 && me) {
      socket?.close();
      me = null;
      view = "login";
    }
    error = err instanceof ApiError ? err.message : "网络异常";
  }

  function notify(message) {
    toast = message;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toast = "";
    }, 4000);
  }

  async function run(fn) {
    error = "";
    try {
      await fn();
    } catch (err) {
      showError(err);
    }
  }

  async function boot() {
    const current = route();
    if (current.name === "register") register.token = current.token;
    try {
      me = await api("/api/me");
    } catch (err) {
      view = current.name === "register" ? "register" : "login";
      if (!(err instanceof ApiError) || err.status !== 401) showError(err);
      return;
    }
    try {
      await refresh();
      connect();
      await openRoute(current.name === "register" ? { name: "chats" } : current);
    } catch (err) {
      if (view === "boot") view = current.name === "chat" ? "chats" : current.name === "friends" || current.name === "settings" ? current.name : "chats";
      showError(err);
    }
  }

  async function openRoute(current = route()) {
    error = "";
    if (!me) {
      view = current.name === "register" ? "register" : "login";
      if (current.token) register.token = current.token;
      return;
    }
    if (current.name === "chat" && current.id) {
      view = "chat";
      await openChat(current.id);
    } else if (current.name === "friends" || current.name === "settings") {
      view = current.name;
      active = null;
      if (current.name === "friends") await loadFriends();
      if (current.name === "settings") sessions = (await api("/api/me/sessions")).sessions;
    } else {
      view = "chats";
      active = null;
    }
  }

  async function refresh() {
    conversations = (await api("/api/conversations")).conversations;
    const incoming = (await api("/api/friends/requests")).requests.filter((item) => item.incoming).length;
    if (incoming > knownIncoming) notify(`${incoming} 条好友申请`);
    knownIncoming = incoming;
  }

  function connect() {
    if (socket) socket.close();
    const protocol = location.protocol === "https:" ? "wss" : "ws";
    socket = new WebSocket(`${protocol}://${location.host}/api/ws`);
    socket.onmessage = (event) => {
      run(async () => {
        const data = JSON.parse(event.data);
        if (data.type === "message" && data.conversation_id === active) {
          if (!messages.some((item) => item.id === data.message.id)) messages = [...messages, data.message];
          await api(`/api/conversations/${active}/read`, { method: "POST", body: { message_id: data.message.id } });
        } else if (data.type === "recall" && data.conversation_id === active) {
          messages = messages.map((item) => (item.id === data.message_id ? { ...item, recalled: true, body: null, image_id: null } : item));
        }
        if (data.type !== "pong") await refresh();
      });
    };
    socket.onclose = () => {
      if (me) setTimeout(connect, 2000);
    };
  }

  async function submitLogin(event) {
    event.preventDefault();
    error = "";
    try {
      me = await api("/api/login", { method: "POST", body: login });
      login = { username: "", password: "", totp: "", needTotp: false };
      knownIncoming = 0;
      await refresh();
      connect();
      go("#/chats");
    } catch (err) {
      if (err.extra?.totp_required) login.needTotp = true;
      showError(err);
    }
  }

  async function submitRegister(event) {
    event.preventDefault();
    error = "";
    try {
      me = await api("/api/register", { method: "POST", body: register });
      await refresh();
      connect();
      go("#/chats");
    } catch (err) {
      showError(err);
    }
  }

  async function logout() {
    await run(async () => {
      await api("/api/logout", { method: "POST" });
      me = null;
      socket?.close();
      go("#/login");
    });
  }

  async function openChat(id) {
    active = id;
    info = false;
    view = "chat";
    detail = await api(`/api/conversations/${id}`);
    messages = (await api(`/api/conversations/${id}/messages`)).messages;
    const last = messages.at(-1);
    if (last) await api(`/api/conversations/${id}/read`, { method: "POST", body: { message_id: last.id } });
    await refresh();
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
  }

  async function send(event) {
    event?.preventDefault();
    const value = text.trim();
    if (!value || !active) return;
    error = "";
    try {
      const message = await api(`/api/conversations/${active}/messages`, { method: "POST", body: { text: value } });
      text = "";
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refresh();
    } catch (err) {
      showError(err);
    }
  }

  function keydown(event) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  }

  async function upload(event) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file || !active) return;
    if (file.size > 25 * 1024 * 1024) {
      error = "图片不能超过 25MB";
      return;
    }
    const body = new FormData();
    body.append("file", file);
    try {
      const message = await api(`/api/conversations/${active}/images`, { method: "POST", body });
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refresh();
    } catch (err) {
      showError(err);
    }
  }

  async function recall(message) {
    await run(async () => {
      const updated = await api(`/api/messages/${message.id}/recall`, { method: "POST" });
      messages = messages.map((item) => (item.id === updated.id ? updated : item));
    });
  }

  async function hide(message) {
    await run(async () => {
      await api(`/api/messages/${message.id}/hide`, { method: "POST" });
      messages = messages.filter((item) => item.id !== message.id);
    });
  }

  async function loadFriends() {
    await run(async () => {
      friends = (await api("/api/friends")).friends;
      requests = (await api("/api/friends/requests")).requests;
    });
  }

  async function doSearch(event) {
    event.preventDefault();
    await run(async () => {
      found = (await api("/api/users/search", { method: "POST", body: { q: search } })).users;
      if (!found.length) error = "没有找到完全匹配的用户";
    });
  }

  async function addFriend(id) {
    await run(async () => {
      const result = await api("/api/friends/requests", { method: "POST", body: { user_id: id } });
      notify(result.accepted ? "已成为好友" : "申请已发送");
      friends = (await api("/api/friends")).friends;
      requests = (await api("/api/friends/requests")).requests;
    });
  }

  async function accept(id) {
    await run(async () => {
      await api(`/api/friends/requests/${id}/accept`, { method: "POST" });
      friends = (await api("/api/friends")).friends;
      requests = (await api("/api/friends/requests")).requests;
    });
  }

  async function reject(id) {
    await run(async () => {
      await api(`/api/friends/requests/${id}/reject`, { method: "POST" });
      friends = (await api("/api/friends")).friends;
      requests = (await api("/api/friends/requests")).requests;
    });
  }

  async function removeFriend(id) {
    await run(async () => {
      await api(`/api/friends/${id}`, { method: "DELETE" });
      friends = (await api("/api/friends")).friends;
      requests = (await api("/api/friends/requests")).requests;
    });
  }

  async function startDm(id) {
    await run(async () => {
      const result = await api("/api/conversations/dm", { method: "POST", body: { user_id: id } });
      go(`#/chat/${result.id}`);
    });
  }

  async function createGroup(event) {
    event.preventDefault();
    await run(async () => {
      const result = await api("/api/conversations/group", { method: "POST", body: { name: groupName, member_ids: picked } });
      groupName = "";
      picked = [];
      go(`#/chat/${result.id}`);
    });
  }

  function togglePick(id) {
    picked = picked.includes(id) ? picked.filter((item) => item !== id) : [...picked, id];
  }

  async function saveConversation(event) {
    event.preventDefault();
    const data = new FormData(event.target);
    const body = {};
    if (data.get("name")) body.name = data.get("name");
    if (data.get("retention")) body.retention = Number(data.get("retention"));
    await run(async () => {
      await api(`/api/conversations/${active}`, { method: "PATCH", body });
      detail = await api(`/api/conversations/${active}`);
      await refresh();
    });
  }

  async function addMember(event) {
    event.preventDefault();
    const id = Number(new FormData(event.target).get("user_id"));
    await run(async () => {
      await api(`/api/conversations/${active}/members`, { method: "POST", body: { user_id: id } });
      detail = await api(`/api/conversations/${active}`);
    });
  }

  async function kick(id) {
    await run(async () => {
      await api(`/api/conversations/${active}/members/${id}`, { method: "DELETE" });
      detail = await api(`/api/conversations/${active}`);
    });
  }

  async function transfer(id) {
    await run(async () => {
      await api(`/api/conversations/${active}/transfer`, { method: "POST", body: { user_id: id } });
      detail = await api(`/api/conversations/${active}`);
    });
  }

  async function leave() {
    await run(async () => {
      await api(`/api/conversations/${active}/leave`, { method: "POST" });
      active = null;
      go("#/chats");
    });
  }

  async function changePassword(event) {
    event.preventDefault();
    error = "";
    try {
      await api("/api/me/password", { method: "POST", body: passwords });
      passwords = { old_password: "", new_password: "" };
      notify("密码已更新");
    } catch (err) {
      showError(err);
    }
  }

  async function beginTotp() {
    await run(async () => {
      totp = await api("/api/me/totp/begin", { method: "POST" });
    });
  }

  async function enableTotp(event) {
    event.preventDefault();
    await run(async () => {
      await api("/api/me/totp/enable", { method: "POST", body: { code: totpCode } });
      me = await api("/api/me");
      totp = null;
      notify("两步验证已开启");
    });
  }

  async function disableTotp(event) {
    event.preventDefault();
    const data = new FormData(event.target);
    await run(async () => {
      await api("/api/me/totp/disable", { method: "POST", body: { password: data.get("password"), code: data.get("code") } });
      me = await api("/api/me");
      notify("两步验证已关闭");
    });
  }

  async function revokeSession(id) {
    await run(async () => {
      await api(`/api/me/sessions/${id}`, { method: "DELETE" });
      sessions = (await api("/api/me/sessions")).sessions;
    });
  }

  function keyBytes(value) {
    const padded = value.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((value.length + 3) % 4);
    const raw = atob(padded);
    return Uint8Array.from(raw, (char) => char.charCodeAt(0));
  }

  async function enablePush() {
    await run(async () => {
      const registration = await navigator.serviceWorker.ready;
      const permission = await Notification.requestPermission();
      if (permission !== "granted") {
        error = "没有获得通知权限";
        return;
      }
      const vapid = await api("/api/push/vapid");
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: keyBytes(vapid.publicKey),
      });
      await api("/api/push/subscribe", { method: "POST", body: subscription.toJSON() });
      notify("推送已开启");
    });
  }

  onMount(() => {
    boot();
    const timer = setInterval(() => {
      if (me) refresh().catch(() => {});
    }, 20000);
    const onHash = () => openRoute().catch(showError);
    window.addEventListener("hashchange", onHash);
    return () => {
      clearInterval(timer);
      clearTimeout(toastTimer);
      window.removeEventListener("hashchange", onHash);
      socket?.close();
    };
  });
</script>

{#if view === "boot"}
  <p class="card">正在打开…</p>
{:else if view === "login"}
  <form class="card" onsubmit={submitLogin}>
    <h1>登录</h1>
    <label>用户名 <input bind:value={login.username} autocomplete="username" required /></label>
    <label>密码 <input type="password" bind:value={login.password} autocomplete="current-password" required /></label>
    {#if login.needTotp}
      <label>验证码 <input bind:value={login.totp} inputmode="numeric" autocomplete="one-time-code" required /></label>
    {/if}
    <p class="error">{error}</p>
    <button>登录</button>
  </form>
{:else if view === "register"}
  <form class="card" onsubmit={submitRegister}>
    <h1>接受邀请</h1>
    <label>用户名 <input bind:value={register.username} autocomplete="username" required /></label>
    <label>显示名称 <input bind:value={register.display_name} required /></label>
    <label>密码 <input type="password" bind:value={register.password} autocomplete="new-password" required /></label>
    <p class="muted">密码至少 10 位。用户名注册后不能修改。</p>
    <p class="error">{error}</p>
    <button>注册并登录</button>
  </form>
{:else}
  <div class="app" class:chat-open={view === "chat"}>
    <header class="top">
      <strong>聊天室</strong>
      <nav>
        <button class="tab" class:on={view === "chats" || view === "chat"} onclick={() => go("#/chats")}>消息</button>
        <button class="tab" class:on={view === "friends"} onclick={() => go("#/friends")}>好友</button>
        <button class="tab" class:on={view === "settings"} onclick={() => go("#/settings")}>设置</button>
      </nav>
      <span class="grow"></span>
      <button class="ghost" onclick={logout}>退出</button>
    </header>
    {#if toast}<p class="toast">{toast}</p>{/if}
    <div class="body">
      {#if view === "chats" || view === "chat"}
        <aside class="side">
          {#each conversations as item (item.id)}
            <button class="item" class:on={item.id === active} onclick={() => go(`#/chat/${item.id}`)}>
              <b>{item.title}</b>
              {#if item.unread}<span class="muted"> {item.unread}</span>{/if}
              <br /><small>{item.last_preview || "还没有消息"}</small>
            </button>
          {:else}
            <p class="muted" style="padding:12px">还没有会话。先到好友页加人或建群。</p>
          {/each}
        </aside>
        <section class="chat">
          {#if detail && active}
            <div class="top">
              {#if view === "chat"}<button class="ghost" onclick={() => go("#/chats")}>返回</button>{/if}
              <b class="grow">{detail.title}</b>
              <button class="ghost" onclick={async () => { info = !info; if (info) await loadFriends(); }}>{info ? "关闭" : "详情"}</button>
            </div>
            {#if info}
              <form class="page" onsubmit={saveConversation}>
                <p class="muted">消息会在 {retentionLabel(detail.retention)} 后自动删除，最长 30 天。</p>
                {#if detail.kind === "group" && detail.role === "owner"}
                  <input name="name" value={detail.title} maxlength="40" />
                {/if}
                {#if detail.kind === "dm" || detail.role === "owner"}
                  <select name="retention">
                    {#each RETENTION as item}
                      <option value={item[0]} selected={item[0] === detail.retention}>{item[1]}</option>
                    {/each}
                  </select>
                  <button>保存</button>
                {/if}
              </form>
              {#if detail.members}
                {#each detail.members as member}
                  <div class="person">
                    <span>{member.display_name} <small class="muted">@{member.username} · {member.role === "owner" ? "群主" : "成员"}</small></span>
                    {#if detail.role === "owner" && member.id !== me.id}
                      <span class="row">
                        <button type="button" class="ghost" onclick={() => transfer(member.id)}>转让</button>
                        <button type="button" class="danger" onclick={() => kick(member.id)}>移出</button>
                      </span>
                    {/if}
                  </div>
                {/each}
                {#if detail.kind === "group"}
                  <form class="row" onsubmit={addMember}>
                    <select name="user_id">
                      {#each friends.filter((friend) => !detail.members.some((member) => member.id === friend.id)) as friend}
                        <option value={friend.id}>{friend.display_name}</option>
                      {/each}
                    </select>
                    <button>拉好友进群</button>
                  </form>
                  <button type="button" class="danger" onclick={leave}>退出群聊</button>
                {/if}
              {/if}
            {:else}
              <div class="messages" bind:this={scroller}>
                {#each messages as message (message.id)}
                  <article class="bubble" class:mine={message.mine}>
                    <small>{message.sender_display_name} · {fmt(message.created_at)}</small>
                    {#if message.recalled}
                      <p class="muted">消息已撤回</p>
                    {:else if message.kind === "image"}
                      <button type="button" class="pic" onclick={() => (preview = `/api/images/${message.image_id}`)}><img src={`/api/images/${message.image_id}/thumb`} alt="图片" /></button>
                    {:else}
                      <p>{message.body}</p>
                    {/if}
                    <span class="row">
                      {#if message.mine && !message.recalled && message.kind !== "system"}
                        <button class="ghost" onclick={() => recall(message)}>撤回</button>
                      {/if}
                      <button class="ghost" onclick={() => hide(message)}>删除</button>
                    </span>
                  </article>
                {/each}
              </div>
              {#if error}<p class="error" style="padding:0 12px">{error}</p>{/if}
              {#if detail.can_send}
                <form class="composer" onsubmit={send}>
                  <textarea bind:value={text} onkeydown={keydown} maxlength="4000" placeholder="输入消息，Enter 发送"></textarea>
                  <label class="ghost" style="padding:8px 12px;border-radius:8px">图片<input type="file" accept="image/jpeg,image/png,image/webp,image/gif" onchange={upload} hidden /></label>
                  <button>发送</button>
                </form>
              {:else}
                <p class="banner">你们还不是好友，不能继续发送。</p>
              {/if}
            {/if}
          {:else}
            <p class="muted" style="padding:16px">选择一个会话。</p>
          {/if}
        </section>
      {:else if view === "friends"}
        <section class="page">
          <form class="row" onsubmit={doSearch}>
            <input class="grow" bind:value={search} placeholder="完整用户名或 UID" />
            <button>搜索</button>
          </form>
          <p class="error">{error}</p>
          {#each found as user}
            <div class="person"><span>{user.display_name} <small class="muted">@{user.username} · {user.uid}</small></span><button onclick={() => addFriend(user.id)}>加好友</button></div>
          {/each}
          <h2>申请</h2>
          {#each requests as item}
            <div class="person">
              <span>{item.incoming ? "来自" : "发给"} {item.user.display_name}</span>
              {#if item.incoming}<span class="row"><button onclick={() => accept(item.id)}>同意</button><button class="ghost" onclick={() => reject(item.id)}>拒绝</button></span>{/if}
            </div>
          {:else}<p class="muted">没有待处理的申请。</p>{/each}
          <h2>好友</h2>
          {#each friends as friend}
            <div class="person">
              <span>{friend.display_name} <small class="muted">@{friend.username}{friend.online ? " · 在线" : ""}</small></span>
              <span class="row"><button onclick={() => startDm(friend.id)}>私聊</button><button class="danger" onclick={() => removeFriend(friend.id)}>删除</button></span>
            </div>
          {/each}
          <h2>新建群聊</h2>
          <form class="stack" onsubmit={createGroup}>
            <input bind:value={groupName} placeholder="群名称" maxlength="40" required />
            {#each friends as friend}
              <label><input type="checkbox" checked={picked.includes(friend.id)} onchange={() => togglePick(friend.id)} /> {friend.display_name}</label>
            {/each}
            <button>创建</button>
          </form>
        </section>
      {:else}
        <section class="page">
          <h1>{me.display_name}</h1>
          <p>用户名 <b>@{me.username}</b> · UID <b>{me.uid}</b></p>
          <p class="error">{error}</p>
          <h2>修改密码</h2>
          <form class="stack" onsubmit={changePassword}>
            <input type="password" placeholder="原密码" bind:value={passwords.old_password} autocomplete="current-password" required />
            <input type="password" placeholder="新密码" bind:value={passwords.new_password} autocomplete="new-password" required />
            <button>更新密码</button>
          </form>
          <h2>两步验证</h2>
          {#if me.totp_enabled}
            <p>已开启。</p>
            {#if !me.is_admin}
              <form class="stack" onsubmit={disableTotp}>
                <input name="password" type="password" placeholder="密码" required />
                <input name="code" placeholder="验证码" required />
                <button class="danger">关闭</button>
              </form>
            {/if}
          {:else}
            <button onclick={beginTotp}>开始设置</button>
            {#if totp}
              <p>密钥：<code>{totp.secret}</code></p>
              <p class="muted">{totp.otpauth}</p>
              <form class="row" onsubmit={enableTotp}><input bind:value={totpCode} placeholder="验证码" /><button>确认开启</button></form>
            {/if}
          {/if}
          <h2>通知</h2>
          <button onclick={enablePush}>开启新消息提醒</button>
          <p class="muted">提醒里只有“你有新消息”，不包含内容和发送者。</p>
          <h2>登录设备</h2>
          {#each sessions as session}
            <div class="person">
              <span>{session.ip || "未知地址"} <small class="muted">{fmt(session.last_seen)}{session.current ? " · 当前" : ""}</small></span>
              {#if !session.current}<button class="danger" onclick={() => revokeSession(session.id)}>退出</button>{/if}
            </div>
          {/each}
        </section>
      {/if}
    </div>
  </div>
{/if}

{#if preview}
  <button class="overlay" onclick={() => (preview = "")}><img src={preview} alt="原图预览" /></button>
{/if}
