(() => {
  const form = document.getElementById("entry-form");
  if (!form || document.documentElement.dataset.ready) return;
  const errorNode = document.getElementById("entry-error");
  const totpInput = form.querySelector("[name=totp]");
  const note = document.getElementById("entry-note");

  const timer = setTimeout(() => {
    if (document.documentElement.dataset.ready) return;
    if (note) note.textContent = "页面加载较慢。可以在这里登录，或重新加载。";
  }, 4000);

  window.addEventListener("chat-room-ready", () => {
    clearTimeout(timer);
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (errorNode) errorNode.textContent = "";
    const data = Object.fromEntries(new FormData(form).entries());
    if (!data.totp) delete data.totp;
    try {
      const response = await fetch("/api/login", {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(data),
      });
      const text = await response.text();
      let body = {};
      try {
        body = text ? JSON.parse(text) : {};
      } catch {
        body = { error: "服务器返回了无法识别的内容" };
      }
      if (response.ok) {
        location.replace(`${location.pathname}${location.search}#/chats`);
        location.reload();
        return;
      }
      if (body.totp_required) {
        form.parentElement?.classList.add("need-totp");
        totpInput?.removeAttribute("hidden");
        if (totpInput) totpInput.required = true;
      }
      if (errorNode) errorNode.textContent = body.error || "登录失败";
    } catch {
      if (errorNode) errorNode.textContent = "网络异常";
    }
  });
})();
