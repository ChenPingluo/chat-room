export function avatarHue(seed) {
  const text = String(seed || "");
  let hash = 0;
  for (let i = 0; i < text.length; i += 1) hash = (hash * 31 + text.charCodeAt(i)) >>> 0;
  return hash % 360;
}

export function avatarText(name) {
  const value = String(name || "").trim();
  if (!value) return "?";
  const chars = Array.from(value);
  const first = chars[0];
  if (/[A-Za-z]/.test(first) && chars[1] && /[A-Za-z]/.test(chars[1])) return (first + chars[1]).toUpperCase();
  return first.toUpperCase();
}

export function shortTime(ts) {
  if (!ts) return "";
  const date = new Date(ts * 1000);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  if (date.toDateString() === now.toDateString()) return time;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return `昨天 ${time}`;
  if (date.getFullYear() === now.getFullYear()) return `${date.getMonth() + 1}/${date.getDate()}`;
  return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}`;
}
