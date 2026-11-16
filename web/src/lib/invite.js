export const INVITE_LIMIT = 5;

export function quotaSummary(quota) {
  const used = Number(quota?.used || 0);
  const limit = Number(quota?.limit || INVITE_LIMIT);
  const reserved = Number(quota?.reserved || 0);
  const remaining = Number(quota?.remaining ?? Math.max(0, limit - used - reserved));
  return {
    used,
    limit,
    reserved,
    remaining,
    atCap: remaining <= 0,
  };
}

export function invitedLabel(quota) {
  const { used, limit } = quotaSummary(quota);
  return `已邀请 ${used}/${limit}`;
}

export function capExplanation(quota) {
  const { used, reserved, remaining, limit } = quotaSummary(quota);
  if (remaining > 0) return "";
  if (used >= limit) {
    return "邀请名额已用完。每人最多成功邀请 5 人。";
  }
  if (reserved > 0) {
    return "未使用的邀请码占用了名额。作废或等过期后可以再邀请。";
  }
  return "邀请名额已用完，每人最多成功邀请 5 人。";
}

export function pendingInvites(invites, nowSec = Date.now() / 1000) {
  return (invites || []).filter(
    (invite) => !invite.used_by && !invite.revoked && Number(invite.expires_at) > nowSec,
  );
}
