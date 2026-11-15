const KEY = "cr-theme";
const COLORS = { light: "#e7eef2", dark: "#0c1216" };

let current = readStored();
const listeners = new Set();

function readStored() {
  try {
    const value = localStorage.getItem(KEY);
    if (value === "light" || value === "dark" || value === "auto") return value;
  } catch {
    /* 隐私模式等场景下继续跟随系统 */
  }
  return "auto";
}

export function getTheme() {
  return current;
}

export function resolveTheme(mode = current) {
  if (mode === "light" || mode === "dark") return mode;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(mode = current) {
  const resolved = resolveTheme(mode);
  const root = document.documentElement;
  root.dataset.theme = resolved;
  root.dataset.themeChoice = mode;
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", COLORS[resolved]);
}

function emit() {
  for (const listener of listeners) listener(current);
}

export function setTheme(mode) {
  current = mode === "light" || mode === "dark" || mode === "auto" ? mode : "auto";
  try {
    localStorage.setItem(KEY, current);
  } catch {
    /* 记忆失败时本次会话仍然切换 */
  }
  applyTheme(current);
  emit();
}

export function subscribe(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function initTheme() {
  current = readStored();
  applyTheme(current);
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (current === "auto") applyTheme("auto");
  };
  media.addEventListener("change", onChange);
}
