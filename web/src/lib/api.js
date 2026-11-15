export class ApiError extends Error {
  constructor(message, status, extra) {
    super(message);
    this.status = status;
    this.extra = extra || {};
  }
}

export async function api(path, options = {}) {
  const request = { credentials: "same-origin", ...options };
  if (request.body && !(request.body instanceof FormData) && typeof request.body !== "string") {
    request.headers = { "content-type": "application/json", ...(request.headers || {}) };
    request.body = JSON.stringify(request.body);
  }
  const response = await fetch(path, request);
  const text = await response.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { error: "服务器返回了无法识别的内容" };
    }
  }
  if (!response.ok) throw new ApiError(data?.error || "请求失败", response.status, data);
  return data;
}

export function fmt(ts) {
  if (!ts) return "";
  const date = new Date(ts * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())} ${p(date.getHours())}:${p(date.getMinutes())}`;
}

export const RETENTION = [
  [3600, "1 小时"],
  [86400, "1 天"],
  [604800, "7 天"],
  [2592000, "30 天"],
];

export function retentionLabel(secs) {
  return RETENTION.find((item) => item[0] === secs)?.[1] || "30 天";
}
