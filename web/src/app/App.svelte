<script>
  import { onMount } from "svelte";
  import { ApiError, RETENTION, api, fmt, retentionLabel } from "../lib/api.js";
  import Atmosphere from "../lib/Atmosphere.svelte";
  import Icon from "../lib/Icon.svelte";
  import ThemeSwitch from "../lib/ThemeSwitch.svelte";
  import { getTheme, setTheme, subscribe } from "../lib/theme.js";
  import { avatarHue, avatarText, shortTime } from "../lib/ui.js";

  let me = $state(null);
  let view = $state("boot");
  let error = $state("");
  let toast = $state("");
  let conversations = $state([]);
  let active = $state(null);
  let detail = $state(null);
  let messages = $state([]);
  let friends = $state([]);
  let friendsLoaded = $state(false);
  let requests = $state([]);
  let sessions = $state([]);
  let info = $state(false);
  let preview = $state("");
  let text = $state("");
  let login = $state({ username: "", password: "", totp: "", needTotp: false });
  let register = $state({ token: "", username: "", password: "", display_name: "" });
  let registerBlocked = $state(false);
  const loginUrl = `${location.origin}/#/login`;
  let search = $state("");
  let found = $state([]);
  let groupName = $state("");
  let picked = $state([]);
  let passwords = $state({ old_password: "", new_password: "" });
  let totp = $state(null);
  let totpCode = $state("");
  let scroller = $state(null);
  let composer = $state(null);
  let convoQuery = $state("");
  let pendingCount = $state(0);
  let sending = $state(false);
  let uploading = $state(false);
  let freshId = $state(0);
  let theme = $state(getTheme());
  let socket;
  let alive = true;
  let knownIncoming = 0;
  let retryMs = 1000;
  let sawPong = true;
  let chatEpoch = 0;
  let toastTimer;
  let stick = true;

  let unreadTotal = $derived(conversations.reduce((sum, item) => sum + Number(item.unread || 0), 0));
  let visibleConversations = $derived.by(() => {
    const query = convoQuery.trim().toLowerCase();
    if (!query) return conversations;
    return conversations.filter((item) => `${item.title}\n${item.last_preview || ""}`.toLowerCase().includes(query));
  });
  let themeIcon = $derived(theme === "light" ? "sun" : theme === "dark" ? "moon" : "auto");
  let themeLabel = $derived(theme === "light" ? "浅色外观，点击切换" : theme === "dark" ? "深色外观，点击切换" : "跟随系统外观，点击切换");
  let inviteCandidates = $derived.by(() => {
    const members = detail?.members || [];
    return friends.filter((friend) => !members.some((member) => member.id === friend.id));
  });

  function safeDecode(value) {
    try {
      return decodeURIComponent(value);
    } catch {
      return value;
    }
  }

  function targetPath() {
    const hash = location.hash.replace(/^#/, "");
    if (hash.startsWith("/")) return hash;
    const path = location.pathname.replace(/\/+$/, "") || "/";
    if (path === "/login" || path.startsWith("/register/")) return path;
    return "/chats";
  }

  function route() {
    const hash = targetPath();
    if (hash.startsWith("/register/")) return { name: "register", token: safeDecode(hash.slice(10)) };
    if (hash.startsWith("/chat/")) return { name: "chat", id: Number(hash.slice(6)) };
    if (hash === "/friends") return { name: "friends" };
    if (hash === "/settings") return { name: "settings" };
    if (hash === "/login") return { name: "login" };
    return { name: "chats" };
  }

  function canonicalize() {
    const path = location.pathname.replace(/\/+$/, "") || "/";
    if (path === "/login" || path.startsWith("/register/")) {
      const next = `#${path}`;
      if (location.hash !== next) history.replaceState(null, "", `/${location.search}${next}`);
    }
  }

  function go(hash) {
    location.hash = hash;
  }

  function leaveSession() {
    view = "login";
    me = null;
    conversations = [];
    messages = [];
    active = null;
    detail = null;
    info = false;
    friends = [];
    requests = [];
    sessions = [];
    found = [];
    preview = "";
    pendingCount = 0;
    knownIncoming = 0;
    const current = socket;
    socket = undefined;
    current?.close();
    if (location.hash !== "#/login") location.hash = "#/login";
  }

  function showError(err) {
    if (err instanceof ApiError && err.status === 401 && me) leaveSession();
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

  function takeRequests(rows) {
    requests = rows;
    pendingCount = rows.filter((item) => item.incoming).length;
  }

  function cycleTheme() {
    const order = ["auto", "light", "dark"];
    setTheme(order[(order.indexOf(theme) + 1) % order.length]);
  }

  function isGrouped(index) {
    const message = messages[index];
    const prev = messages[index - 1];
    if (!message || !prev) return false;
    if (message.kind === "system" || prev.kind === "system" || message.recalled || prev.recalled) return false;
    const current = message.sender_id ?? message.sender_display_name;
    const previous = prev.sender_id ?? prev.sender_display_name;
    return current === previous && message.mine === prev.mine && message.created_at - prev.created_at < 300;
  }

  function noteScroll() {
    if (!scroller) return;
    stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 140;
  }

  async function copyValue(value, label) {
    try {
      await navigator.clipboard.writeText(value);
      notify(`已复制${label}`);
    } catch {
      error = "复制失败，请手动选择";
    }
  }

  function deviceText(session) {
    const agent = (session.user_agent || "").replace(/\s+/g, " ").trim();
    if (!agent) return "";
    return agent.length > 80 ? `${agent.slice(0, 80)}…` : agent;
  }

  async function boot() {
    canonicalize();
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
      if (current.name === "register" && current.token && current.token !== register.token) registerBlocked = false;
      if (current.token) register.token = current.token;
      if (current.name !== "register") registerBlocked = false;
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
    const incomingRows = (await api("/api/friends/requests")).requests;
    const incoming = incomingRows.filter((item) => item.incoming).length;
    pendingCount = incoming;
    if (incoming > knownIncoming) notify(`${incoming} 条好友申请`);
    knownIncoming = incoming;
  }

  async function resync() {
    if (!me) return;
    const ticket = chatEpoch;
    const conversation = active;
    await refresh();
    if (ticket !== chatEpoch || conversation !== active || !conversation) return;
    const rows = (await api(`/api/conversations/${conversation}/messages`)).messages;
    if (ticket !== chatEpoch || conversation !== active) return;
    const merged = new Map(messages.map((item) => [item.id, item]));
    for (const item of rows) merged.set(item.id, item);
    messages = [...merged.values()].sort((a, b) => a.id - b.id);
  }

  function alertIncoming(message) {
    notify(`${message.sender_display_name || "有人"}发来一条消息`);
    if (document.hidden && typeof Notification !== "undefined" && Notification.permission === "granted") {
      navigator.serviceWorker?.ready
        .then((registration) => registration.showNotification("新消息", { body: "你有新消息", icon: "/icon-192.png" }))
        .catch(() => {});
    }
  }

  function connect() {
    if (socket) {
      socket.onclose = null;
      socket.close();
    }
    const protocol = location.protocol === "https:" ? "wss" : "ws";
    const current = new WebSocket(`${protocol}://${location.host}/api/ws`);
    socket = current;
    sawPong = true;
    current.onopen = () => {
      retryMs = 1000;
      resync().catch(() => {});
    };
    current.onmessage = (event) => {
      run(async () => {
        const data = JSON.parse(event.data);
        if (data.type === "pong") {
          sawPong = true;
          return;
        }
        if (data.type === "message") {
          const message = { ...data.message, mine: data.message.sender_id === me?.id };
          if (data.conversation_id === active) {
            if (!messages.some((item) => item.id === message.id)) {
              freshId = message.id;
              messages = [...messages, message];
            }
            await api(`/api/conversations/${active}/read`, { method: "POST", body: { message_id: message.id } });
          } else if (!message.mine) {
            alertIncoming(message);
          }
        } else if (data.type === "recall" && data.conversation_id === active) {
          messages = messages.map((item) => (item.id === data.message_id ? { ...item, recalled: true, body: null, image_id: null } : item));
        }
        await refresh();
      });
    };
    current.onclose = () => {
      if (current !== socket || !me || !alive) return;
      setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 15000);
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
    registerBlocked = false;
    try {
      me = await api("/api/register", { method: "POST", body: register });
      await refresh();
      connect();
      go("#/chats");
    } catch (err) {
      registerBlocked = err instanceof ApiError && err.message === "邀请无效";
      showError(err);
    }
  }

  async function logout() {
    error = "";
    try {
      await api("/api/logout", { method: "POST" });
    } catch (err) {
      if (!(err instanceof ApiError) || err.status !== 401) {
        showError(err);
        return;
      }
    }
    leaveSession();
  }

  async function openChat(id) {
    chatEpoch += 1;
    active = id;
    info = false;
    view = "chat";
    stick = true;
    freshId = 0;
    detail = null;
    messages = [];
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
    if (!value || !active || sending) return;
    sending = true;
    error = "";
    try {
      const message = await api(`/api/conversations/${active}/messages`, { method: "POST", body: { text: value } });
      text = "";
      freshId = message.id;
      stick = true;
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refresh();
    } catch (err) {
      showError(err);
    } finally {
      sending = false;
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
    if (!file || !active || uploading) return;
    if (file.size > 25 * 1024 * 1024) {
      error = "图片不能超过 25MB";
      return;
    }
    const body = new FormData();
    body.append("file", file);
    uploading = true;
    error = "";
    try {
      const message = await api(`/api/conversations/${active}/images`, { method: "POST", body });
      freshId = message.id;
      stick = true;
      if (!messages.some((item) => item.id === message.id)) messages = [...messages, message];
      await refresh();
    } catch (err) {
      showError(err);
    } finally {
      uploading = false;
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
      takeRequests((await api("/api/friends/requests")).requests);
      friendsLoaded = true;
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
      takeRequests((await api("/api/friends/requests")).requests);
    });
  }

  async function accept(id) {
    await run(async () => {
      await api(`/api/friends/requests/${id}/accept`, { method: "POST" });
      friends = (await api("/api/friends")).friends;
      takeRequests((await api("/api/friends/requests")).requests);
    });
  }

  async function reject(id) {
    await run(async () => {
      await api(`/api/friends/requests/${id}/reject`, { method: "POST" });
      friends = (await api("/api/friends")).friends;
      takeRequests((await api("/api/friends/requests")).requests);
    });
  }

  async function removeFriend(id) {
    await run(async () => {
      await api(`/api/friends/${id}`, { method: "DELETE" });
      friends = (await api("/api/friends")).friends;
      takeRequests((await api("/api/friends/requests")).requests);
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
      notify("会话设置已保存");
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

  async function toggleInfo() {
    info = !info;
    if (info) await loadFriends();
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

  $effect(() => {
    document.title = unreadTotal ? `(${unreadTotal}) 聊天室` : "聊天室";
  });

  $effect(() => {
    const marker = `${messages.length}:${freshId}`;
    if (!stick || !scroller || !messages.length) return;
    void marker;
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
  });

  $effect(() => {
    text;
    if (!composer) return;
    composer.style.height = "auto";
    composer.style.height = `${Math.min(composer.scrollHeight, 160)}px`;
  });

  onMount(() => {
    const stopTheme = subscribe((value) => {
      theme = value;
    });
    const onKey = (event) => {
      if (event.key !== "Escape") return;
      if (preview) preview = "";
      else if (info) info = false;
    };
    boot();
    const beat = setInterval(() => {
      if (!socket || socket.readyState !== WebSocket.OPEN) return;
      if (!sawPong) {
        socket.close();
        return;
      }
      sawPong = false;
      socket.send('{"type":"ping"}');
    }, 25000);
    const timer = setInterval(() => {
      if (!me) return;
      const task = socket?.readyState === WebSocket.OPEN ? refresh() : resync();
      task.catch(() => {});
    }, 20000);
    const onHash = () => openRoute().catch(showError);
    const onVisible = () => {
      if (document.visibilityState === "visible" && me) resync().catch(() => {});
    };
    window.addEventListener("hashchange", onHash);
    window.addEventListener("keydown", onKey);
    window.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      stopTheme();
      clearInterval(timer);
      clearInterval(beat);
      clearTimeout(toastTimer);
      window.removeEventListener("hashchange", onHash);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("visibilitychange", onVisible);
      socket?.close();
    };
  });
</script>

{#snippet avatar(name, hueSeed, online, group)}
  <span class="avatar" class:group style="--hue: {avatarHue(hueSeed || name)}">
    {avatarText(name)}
    {#if online}<span class="dot"></span>{/if}
  </span>
{/snippet}

{#snippet badge(value)}
  {#if value > 0}<span class="count">{value > 99 ? "99+" : value}</span>{/if}
{/snippet}

<Atmosphere />
<svelte:boundary onerror={(err) => console.error(err)}>
{#if view === "boot" || view === "login" || view === "register"}
  <main class="gate">
    {#if view === "boot"}
      <section class="card glass boot-card" aria-busy="true" aria-label="正在打开">
        <div class="brand">
          <span class="sk logo"></span>
          <div class="sk-stack">
            <span class="sk title"></span>
            <span class="sk line"></span>
          </div>
        </div>
        <span class="sk line"></span>
        <span class="sk short"></span>
      </section>
    {:else if view === "login"}
      <form class="card glass" onsubmit={submitLogin}>
        <div class="card-top">
          <div class="brand">
            <img class="logo" src="/icon.svg" alt="" />
            <div>
              <h1>登录</h1>
              <p>只和你邀请的人说话</p>
            </div>
          </div>
          <ThemeSwitch compact />
        </div>
        <label class="field">用户名 <input bind:value={login.username} autocomplete="username" required /></label>
        <label class="field">密码 <input type="password" bind:value={login.password} autocomplete="current-password" required /></label>
        {#if login.needTotp}
          <label class="field reveal">验证码 <input bind:value={login.totp} inputmode="numeric" autocomplete="one-time-code" required /></label>
        {/if}
        <p class="error" role="alert">{error}</p>
        <button class="submit">登录</button>
        <p class="muted">有邀请链接？直接打开链接即可注册。</p>
        <p class="hint">这就是登录入口。地址是 <code>{loginUrl}</code></p>
      </form>
    {:else}
      <form class="card glass" onsubmit={submitRegister}>
        <div class="card-top">
          <div class="brand">
            <img class="logo" src="/icon.svg" alt="" />
            <div>
              <h1>接受邀请</h1>
              <p>创建后即可进入聊天室</p>
            </div>
          </div>
          <ThemeSwitch compact />
        </div>
        {#if registerBlocked}
          <div class="notice" role="alert">
            <p>这个邀请链接已经用过、过期或作废了。</p>
            <button type="button" onclick={() => go("#/login")}>去登录</button>
          </div>
        {:else}
          <label class="field">用户名 <input bind:value={register.username} autocomplete="username" required /></label>
          <label class="field">显示名称 <input bind:value={register.display_name} required /></label>
          <label class="field">密码 <input type="password" bind:value={register.password} autocomplete="new-password" required /></label>
          <p class="muted">密码至少 10 位。用户名注册后不能修改。</p>
          <p class="error" role="alert">{error}</p>
          <button class="submit">注册并登录</button>
        {/if}
        <p class="muted">已有账号？ <button type="button" class="linkish" onclick={() => go("#/login")}>去登录</button></p>
      </form>
    {/if}
  </main>
{:else}
  <div class="shell pane-{view}" class:chat-open={view === "chat"}>
    <aside class="rail glass">
      <img class="logo" src="/icon.svg" alt="聊天室" />
      <nav aria-label="主导航">
        <button type="button" class="nav-btn" class:on={view === "chats" || view === "chat"} aria-current={view === "chats" || view === "chat" ? "page" : undefined} aria-label="消息" onclick={() => go("#/chats")}>
          <Icon name="chat" />
          {@render badge(unreadTotal)}
        </button>
        <button type="button" class="nav-btn" class:on={view === "friends"} aria-current={view === "friends" ? "page" : undefined} aria-label="好友" onclick={() => go("#/friends")}>
          <Icon name="users" />
          {@render badge(pendingCount)}
        </button>
        <button type="button" class="nav-btn" class:on={view === "settings"} aria-current={view === "settings" ? "page" : undefined} aria-label="设置" onclick={() => go("#/settings")}>
          <Icon name="settings" />
        </button>
      </nav>
      <div class="rail-foot">
        <button type="button" class="nav-btn" title={themeLabel} aria-label={themeLabel} onclick={cycleTheme}><Icon name={themeIcon} /></button>
        <button type="button" class="nav-btn" aria-label="退出" onclick={logout}><Icon name="logout" /></button>
      </div>
    </aside>
    <div class="stage" class:solo={view === "friends" || view === "settings"} class:show-drawer={info && view === "chat"}>
      {#if view === "chats" || view === "chat"}
        <aside class="convos">
          <div class="pane-head">
            <div class="pane-title">
              <h2>消息</h2>
              <small class="muted">{conversations.length} 个会话</small>
            </div>
            <label class="search">
              <Icon name="search" size={18} />
              <input bind:value={convoQuery} placeholder="搜索会话" autocomplete="off" aria-label="搜索会话" />
            </label>
          </div>
          <div class="list">
            {#each visibleConversations as item, index (item.id)}
              <button type="button" class="item" class:on={item.id === active} style="--i: {Math.min(index, 12)}" onclick={() => go(`#/chat/${item.id}`)}>
                {@render avatar(item.title, item.title, false, item.kind === "group")}
                <span class="item-body">
                  <span class="item-top">
                    <strong class:unread={item.unread}>{item.title}</strong>
                    <small class="muted">{shortTime(item.last_at)}</small>
                  </span>
                  <span class="item-sub">
                    <small>{item.last_preview || "还没有消息"}</small>
                    {@render badge(item.unread)}
                  </span>
                </span>
              </button>
            {:else}
              <div class="empty-inline">
                <p class="muted">{convoQuery ? "没有匹配的会话" : "还没有会话。先到好友页加人或建群。"}</p>
                {#if !convoQuery}<button type="button" class="ghost" onclick={() => go("#/friends")}>去加好友</button>{/if}
              </div>
            {/each}
          </div>
        </aside>
        <section class="thread">
          {#if detail && active}
            <header class="topbar glass">
              <button type="button" class="icon-btn only-mobile" aria-label="返回会话列表" onclick={() => go("#/chats")}><Icon name="back" /></button>
              {@render avatar(detail.title, detail.title, false, detail.kind === "group")}
              <div class="who-text">
                <strong>{detail.title}</strong>
                <span class="tags">
                  <span class="tag">{detail.kind === "group" ? "群聊" : "私聊"}</span>
                  {#if detail.kind === "group"}<span class="tag quiet">{detail.members?.length || 0} 人</span>{/if}
                  <span class="tag quiet">{retentionLabel(detail.retention)}后删除</span>
                  {#if !detail.can_send}<span class="tag warn">无法发送</span>{/if}
                </span>
              </div>
              <button type="button" class="ghost" aria-expanded={info} onclick={toggleInfo}>{info ? "关闭" : "详情"}</button>
            </header>
            <div class="messages" bind:this={scroller} onscroll={noteScroll}>
              {#each messages as message, index (message.id)}
                {#if message.kind === "system"}
                  <article class="system" class:fresh={message.id === freshId}>
                    <p>{message.recalled ? "消息已撤回" : message.body}</p>
                    <button type="button" class="ghost" onclick={() => hide(message)}>删除</button>
                  </article>
                {:else}
                  <article class="bubble" class:mine={message.mine} class:grouped={isGrouped(index)} class:fresh={message.id === freshId}>
                    <header class="meta">
                      {#if !isGrouped(index)}<span>{message.sender_display_name}</span>{/if}
                      <time title={fmt(message.created_at)}>{shortTime(message.created_at) || fmt(message.created_at)}</time>
                    </header>
                    {#if message.recalled}
                      <p class="muted">消息已撤回</p>
                    {:else if message.kind === "image"}
                      <button type="button" class="pic" onclick={() => (preview = `/api/images/${message.image_id}`)}><img src={`/api/images/${message.image_id}/thumb`} alt="图片" /></button>
                    {:else}
                      <p>{message.body}</p>
                    {/if}
                    <div class="actions">
                      {#if message.mine && !message.recalled && message.kind !== "system"}
                        <button type="button" class="ghost" onclick={() => recall(message)}>撤回</button>
                      {/if}
                      <button type="button" class="ghost" onclick={() => hide(message)}>删除</button>
                    </div>
                  </article>
                {/if}
              {/each}
            </div>
            {#if error}<p class="error thread-error" role="alert">{error}</p>{/if}
            {#if uploading}<p class="upload-note">正在处理图片…</p>{/if}
            {#if detail.can_send}
              <form class="composer glass" onsubmit={send}>
                <textarea bind:this={composer} bind:value={text} onkeydown={keydown} rows="1" maxlength="4000" placeholder="输入消息，Enter 发送" aria-label="消息"></textarea>
                <label class="icon-btn" title="发送图片">
                  <Icon name="image" />
                  <span class="sr">发送图片</span>
                  <input type="file" accept="image/jpeg,image/png,image/webp,image/gif" onchange={upload} hidden disabled={uploading} />
                </label>
                <button class="send" aria-label="发送" disabled={sending || uploading || !text.trim()}><Icon name="send" /></button>
              </form>
            {:else}
              <p class="banner">你们还不是好友，不能继续发送。</p>
            {/if}
          {:else}
            <div class="empty">
              <img class="logo" src="/icon.svg" alt="" />
              <h2>{active ? "正在打开会话…" : "选择一个会话"}</h2>
              <p>{error || "从左侧选一个聊天，或去好友页开始新的对话。"}</p>
              {#if !active}<button type="button" class="ghost" onclick={() => go("#/friends")}>去加好友</button>{/if}
            </div>
          {/if}
        </section>
        {#if info && detail}
          <button type="button" class="shade" aria-label="关闭详情" onclick={() => (info = false)}></button>
          <aside class="drawer glass" aria-label="会话详情">
            <div class="drawer-head">
              <h2>会话详情</h2>
              <button type="button" class="icon-btn" aria-label="关闭详情" onclick={() => (info = false)}><Icon name="close" /></button>
            </div>
            <p class="muted">消息会在 {retentionLabel(detail.retention)} 后自动删除，最长 30 天。</p>
            {#if detail.kind === "dm" || detail.role === "owner"}
              <form class="stack" onsubmit={saveConversation}>
                {#if detail.kind === "group"}
                  <label class="field">群名称 <input name="name" value={detail.title} maxlength="40" /></label>
                {/if}
                <label class="field">保留时间
                  <select name="retention">
                    {#each RETENTION as item (item[0])}
                      <option value={item[0]} selected={item[0] === detail.retention}>{item[1]}</option>
                    {/each}
                  </select>
                </label>
                <button>保存</button>
              </form>
            {/if}
            {#if detail.members}
              <div class="stack">
                <h2>成员</h2>
                {#each detail.members as member (member.id)}
                  <div class="person">
                    {@render avatar(member.display_name, member.username, member.online, false)}
                    <span class="person-body">
                      <strong>{member.display_name}</strong>
                      <small class="muted">@{member.username}{#if detail.kind === "group"} · {member.role === "owner" ? "群主" : "成员"}{/if}{#if member.online} · 在线{/if}</small>
                    </span>
                    {#if detail.role === "owner" && member.id !== me?.id}
                      <span class="actions">
                        <button type="button" class="ghost" onclick={() => transfer(member.id)}>转让</button>
                        <button type="button" class="danger" onclick={() => kick(member.id)}>移出</button>
                      </span>
                    {/if}
                  </div>
                {/each}
              </div>
              {#if detail.kind === "group"}
                {#if !friendsLoaded}
                  <p class="muted">正在读取好友…</p>
                {:else if inviteCandidates.length}
                  <form class="row" onsubmit={addMember}>
                    <select name="user_id" aria-label="选择好友">
                      {#each inviteCandidates as friend (friend.id)}
                        <option value={friend.id}>{friend.display_name}</option>
                      {/each}
                    </select>
                    <button>拉好友进群</button>
                  </form>
                {:else}
                  <p class="muted">可以邀请的好友都已经在群里。</p>
                {/if}
                <button type="button" class="danger" onclick={leave}>退出群聊</button>
              {/if}
            {/if}
          </aside>
        {/if}
      {:else if view === "friends"}
        <section class="canvas">
          <div class="board columns">
            <section class="panel wide">
              <h2>查找用户</h2>
              <form class="row" onsubmit={doSearch}>
                <input class="grow" bind:value={search} placeholder="完整用户名或 UID" aria-label="完整用户名或 UID" />
                <button>搜索</button>
              </form>
              <p class="error" role="alert">{error}</p>
              {#each found as user, index (user.id)}
                <div class="person" style="--i: {Math.min(index, 10)}">
                  {@render avatar(user.display_name, user.username, user.online, false)}
                  <span class="person-body"><strong>{user.display_name}</strong><br /><small class="muted">@{user.username} · {user.uid}</small></span>
                  <button type="button" onclick={() => addFriend(user.id)}>加好友</button>
                </div>
              {/each}
            </section>
            <section class="panel">
              <h2>申请 {@render badge(pendingCount)}</h2>
              {#each requests as item, index (item.id)}
                <div class="person" style="--i: {Math.min(index, 10)}">
                  {@render avatar(item.user.display_name, item.user.username, item.user.online, false)}
                  <span class="person-body"><strong>{item.incoming ? "来自" : "发给"} {item.user.display_name}</strong><br /><small class="muted">@{item.user.username}</small></span>
                  {#if item.incoming}
                    <span class="actions">
                      <button type="button" onclick={() => accept(item.id)}>同意</button>
                      <button type="button" class="ghost" onclick={() => reject(item.id)}>拒绝</button>
                    </span>
                  {/if}
                </div>
              {:else}
                <p class="muted">没有待处理的申请。</p>
              {/each}
            </section>
            <section class="panel">
              <h2>好友</h2>
              {#each friends as friend, index (friend.id)}
                <div class="person" style="--i: {Math.min(index, 10)}">
                  {@render avatar(friend.display_name, friend.username, friend.online, false)}
                  <span class="person-body">
                    <strong>{friend.display_name}</strong>
                    <br /><small class="muted">@{friend.username}{friend.online ? " · 在线" : ""}</small>
                  </span>
                  <span class="actions">
                    <button type="button" onclick={() => startDm(friend.id)}>私聊</button>
                    <button type="button" class="danger" onclick={() => removeFriend(friend.id)}>删除</button>
                  </span>
                </div>
              {:else}
                <p class="muted">还没有好友。</p>
              {/each}
            </section>
            <section class="panel wide">
              <h2>新建群聊</h2>
              <form class="stack" onsubmit={createGroup}>
                <input bind:value={groupName} placeholder="群名称" maxlength="40" aria-label="群名称" required />
                <div class="row">
                  {#each friends as friend (friend.id)}
                    <label class="check" class:on={picked.includes(friend.id)}>
                      <input type="checkbox" checked={picked.includes(friend.id)} onchange={() => togglePick(friend.id)} />
                      {friend.display_name}
                    </label>
                  {:else}
                    <p class="muted">先添加好友，才能拉进群。</p>
                  {/each}
                </div>
                <button>创建</button>
              </form>
            </section>
          </div>
        </section>
      {:else if me}
        <section class="canvas">
          <div class="board columns">
            <section class="panel wide account">
              <div class="who">
                {@render avatar(me.display_name, me.username, false, false)}
                <div class="who-text">
                  <h1>{me.display_name}</h1>
                  <p class="muted">@{me.username}</p>
                </div>
              </div>
              <div class="row">
                <code class="uid">{me.uid}</code>
                <button type="button" class="ghost" onclick={() => copyValue(me.uid, " UID")}>复制 UID</button>
                <button type="button" class="ghost" onclick={() => copyValue(`@${me.username}`, "用户名")}>复制用户名</button>
                <button type="button" class="danger with-icon" onclick={logout}><Icon name="logout" size={18} /> 退出登录</button>
              </div>
              <p class="error" role="alert">{error}</p>
            </section>
            <section class="panel">
              <h2>外观</h2>
              <p class="muted">浅色、深色，或跟随系统。选择会记在这台设备上。</p>
              <ThemeSwitch />
            </section>
            <section class="panel">
              <h2>修改密码</h2>
              <form class="stack" onsubmit={changePassword}>
                <input type="password" placeholder="原密码" aria-label="原密码" bind:value={passwords.old_password} autocomplete="current-password" required />
                <input type="password" placeholder="新密码" aria-label="新密码" bind:value={passwords.new_password} autocomplete="new-password" required />
                <button>更新密码</button>
              </form>
            </section>
            <section class="panel">
              <h2>两步验证</h2>
              {#if me.totp_enabled}
                <p><span class="tag ok">已开启</span></p>
                {#if !me.is_admin}
                  <form class="stack" onsubmit={disableTotp}>
                    <input name="password" type="password" placeholder="密码" aria-label="密码" required />
                    <input name="code" placeholder="验证码" aria-label="验证码" required />
                    <button class="danger">关闭</button>
                  </form>
                {:else}
                  <p class="muted">管理员账号需要保持两步验证。</p>
                {/if}
              {:else}
                <button type="button" onclick={beginTotp}>开始设置</button>
                {#if totp}
                  <p>密钥：<code>{totp.secret}</code></p>
                  <p class="muted">{totp.otpauth}</p>
                  <form class="row" onsubmit={enableTotp}><input bind:value={totpCode} placeholder="验证码" aria-label="验证码" /><button>确认开启</button></form>
                {/if}
              {/if}
            </section>
            <section class="panel">
              <h2>通知</h2>
              <button type="button" onclick={enablePush}>开启新消息提醒</button>
              <p class="muted">提醒里只有“你有新消息”，不包含内容和发送者。</p>
            </section>
            <section class="panel wide">
              <h2>登录设备</h2>
              {#each sessions as session, index (session.id)}
                <div class="person" style="--i: {Math.min(index, 10)}">
                  <span class="avatar" style="--hue: 188"><Icon name="devices" size={18} /></span>
                  <span class="person-body">
                    <strong>{session.ip || "未知地址"}{session.current ? " · 当前" : ""}</strong>
                    <br /><small class="muted">{fmt(session.last_seen)}{deviceText(session) ? ` · ${deviceText(session)}` : ""}</small>
                  </span>
                  {#if !session.current}<button type="button" class="danger" onclick={() => revokeSession(session.id)}>退出</button>{/if}
                </div>
              {/each}
            </section>
          </div>
        </section>
      {:else}
        <section class="canvas">
          <div class="empty">
            <h2>正在返回登录</h2>
            <button type="button" onclick={() => go("#/login")}>去登录</button>
          </div>
        </section>
      {/if}
    </div>
    <nav class="tabbar glass" aria-label="主导航">
      <button type="button" class:on={view === "chats" || view === "chat"} onclick={() => go("#/chats")}>
        <Icon name="chat" />
        <span>消息</span>
        {@render badge(unreadTotal)}
      </button>
      <button type="button" class:on={view === "friends"} onclick={() => go("#/friends")}>
        <Icon name="users" />
        <span>好友</span>
        {@render badge(pendingCount)}
      </button>
      <button type="button" class:on={view === "settings"} onclick={() => go("#/settings")}>
        <Icon name="settings" />
        <span>设置</span>
      </button>
    </nav>
  </div>
{/if}

{#if toast}<p class="toast glass" role="status">{toast}</p>{/if}
{#if preview}
  <div
    class="lightbox"
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-label="原图预览"
    onclick={() => (preview = "")}
    onkeydown={(event) => {
      if (event.key === "Escape" || event.key === "Enter" || event.key === " ") preview = "";
    }}
  >
    <button type="button" class="icon-btn lightbox-close" aria-label="关闭预览"><Icon name="close" /></button>
    <img src={preview} alt="原图预览" />
  </div>
{/if}
{#snippet failed(error, reset)}
  <main class="gate">
    <section class="card glass">
      <div class="brand">
        <img class="logo" src="/icon.svg" alt="" />
        <div>
          <h1>页面出错了</h1>
          <p>{error?.message || "可以重新加载，或回到登录页。"}</p>
        </div>
      </div>
      <button type="button" onclick={() => location.reload()}>重新加载</button>
      <button type="button" class="ghost" onclick={() => { leaveSession(); reset(); }}>回到登录页</button>
    </section>
  </main>
{/snippet}
</svelte:boundary>
