export type Me = {
  id: number;
  uid: string;
  username: string;
  is_admin: boolean;
  created_at: number;
};

export type PublicUser = {
  id: number;
  uid: string;
  username: string;
  relation?: string;
};

export type Attachment = {
  id: number;
  width: number;
  height: number;
  size: number;
};

export type Message = {
  id: number;
  conv_id: number;
  sender_id: number;
  kind: 'text' | 'image' | string;
  body: string | null;
  attachment: Attachment | null;
  created_at: number;
  expires_at: number;
  deleted: boolean;
  client_msg_id?: string;
};

export type Member = {
  id: number;
  uid: string;
  username: string;
  role: string;
};

export type Conversation = {
  id: number;
  kind: 'direct' | 'group' | string;
  name: string | null;
  retention_secs: number;
  my_role: string;
  owner_id: number | null;
  members: Member[];
  last_message: Message | null;
  unread: number;
  created_at: number;
};

export type FriendRequest = {
  id: number;
  from_user: PublicUser;
  to_user: PublicUser;
  status: string;
  created_at: number;
};

export type SessionInfo = {
  id: number;
  created_at: number;
  last_seen: number;
  expires_at: number;
  user_agent: string;
  current: boolean;
};

export type Invite = {
  id: number;
  created_at: number;
  expires_at: number;
  revoked: boolean;
  used_at: number | null;
  used_by: PublicUser | null;
};

export type ServerEvent =
  | { type: 'ready'; user_id: number }
  | { type: 'message'; message: Message }
  | { type: 'message_deleted'; conv_id: number; message_id: number }
  | { type: 'friend_request'; request: FriendRequest }
  | { type: 'friendship'; user: PublicUser; status: string }
  | { type: 'conversation'; conversation: Conversation }
  | { type: 'conversation_removed'; conv_id: number }
  | { type: 'error'; error: string; client_msg_id?: string }
  | { type: 'pong' };

export const RETENTIONS = [
  { secs: 3600, label: '1 小时' },
  { secs: 86400, label: '1 天' },
  { secs: 604800, label: '7 天' },
  { secs: 2592000, label: '30 天' },
];

export function retentionLabel(secs: number): string {
  return RETENTIONS.find((item) => item.secs === secs)?.label ?? `${secs} 秒`;
}

export function formatTime(ms: number): string {
  const date = new Date(ms);
  const now = new Date();
  const hm = date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  if (date.toDateString() === now.toDateString()) return hm;
  return `${date.getMonth() + 1}-${date.getDate()} ${hm}`;
}

export function conversationTitle(conversation: Conversation, meId: number): string {
  if (conversation.kind === 'group') return conversation.name || '未命名群';
  const peer = conversation.members.find((member) => member.id !== meId);
  return peer?.username || '私聊';
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has('content-type')) {
    headers.set('content-type', 'application/json');
  }
  const response = await fetch(path, { ...init, headers, credentials: 'same-origin' });
  if (response.status === 204) return undefined as T;
  const data = (await response.json().catch(() => ({}))) as { error?: string };
  if (!response.ok) throw new ApiError(response.status, data.error || '请求失败');
  return data as T;
}

export function connectSocket(onEvent: (event: ServerEvent) => void): () => void {
  let socket: WebSocket | null = null;
  let closed = false;
  let timer = 0;
  let ping = 0;

  const open = () => {
    if (closed) return;
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(`${protocol}//${location.host}/ws`);
    socket.onmessage = (message) => {
      try {
        onEvent(JSON.parse(String(message.data)) as ServerEvent);
      } catch {
        /* 忽略无法解析的帧 */
      }
    };
    socket.onclose = () => {
      window.clearInterval(ping);
      if (!closed) timer = window.setTimeout(open, 1500);
    };
    socket.onopen = () => {
      ping = window.setInterval(() => {
        if (socket?.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: 'ping' }));
        }
      }, 25000);
    };
  };
  open();
  return () => {
    closed = true;
    window.clearTimeout(timer);
    window.clearInterval(ping);
    socket?.close();
  };
}
