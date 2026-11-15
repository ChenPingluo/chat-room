export class ApiError extends Error {
  constructor(message, status, extra) {
    super(message);
    this.status = status;
    this.extra = extra || {};
  }
}

export async function api(path, options = {}) {
  const { timeout = 15000, signal, ...rest } = options;
  const request = { credentials: "same-origin", ...rest };
  if (request.body && !(request.body instanceof FormData) && typeof request.body !== "string") {
    request.headers = { "content-type": "application/json", ...(request.headers || {}) };
    request.body = JSON.stringify(request.body);
  }
  const ctrl = new AbortController();
  const timer = timeout > 0 ? setTimeout(() => ctrl.abort(), timeout) : null;
  if (signal) {
    if (signal.aborted) ctrl.abort();
    else signal.addEventListener("abort", () => ctrl.abort(), { once: true });
  }
  try {
    const response = await fetch(path, { ...request, signal: ctrl.signal });
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
  } catch (err) {
    if (err instanceof ApiError) throw err;
    if (err?.name === "AbortError") throw new ApiError("连接超时", 0);
    throw new ApiError("网络异常", 0);
  } finally {
    if (timer) clearTimeout(timer);
  }
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
