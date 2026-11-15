export function safeDecode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function targetPath(location) {
  const hash = String(location.hash || "").replace(/^#/, "");
  if (hash.startsWith("/")) return hash;
  const path = String(location.pathname || "/").replace(/\/+$/, "") || "/";
  if (path === "/login" || path.startsWith("/register/")) return path;
  if (path === "/" || path === "") return "/login";
  return "/chats";
}

export function parseRoute(location) {
  const hash = targetPath(location);
  if (hash.startsWith("/register/")) return { name: "register", token: safeDecode(hash.slice(10)) };
  if (hash.startsWith("/chat/")) return { name: "chat", id: Number(hash.slice(6)) };
  if (hash === "/friends") return { name: "friends" };
  if (hash === "/settings") return { name: "settings" };
  if (hash === "/login" || hash === "/" || hash === "") return { name: "login" };
  return { name: "chats" };
}

export function guestView(route) {
  return route?.name === "register" ? "register" : "login";
}

export function loginHref(location) {
  const origin = location?.origin || "";
  return `${origin}/#/login`;
}

export function canonicalizeLocation(location, replaceState) {
  const search = location.search || "";
  const path = String(location.pathname || "/").replace(/\/+$/, "") || "/";
  if (path === "/login" || path.startsWith("/register/")) {
    const next = `#${path}`;
    if (location.hash !== next) replaceState(null, "", `/${search}${next}`);
    return;
  }
  if (path !== "/") return;
  const hash = String(location.hash || "");
  if (!hash || hash === "#" || hash === "#/") {
    replaceState(null, "", `/${search}#/login`);
  }
}

export function loginUrl(location, replaceState) {
  if (location.hash !== "#/login") {
    replaceState(null, "", `/${location.search || ""}#/login`);
  }
}
