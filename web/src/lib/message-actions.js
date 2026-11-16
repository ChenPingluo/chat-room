export const ACTION_RECALL = "recall";
export const ACTION_HIDE = "hide";

export const LONG_PRESS_MS = 480;
export const LONG_PRESS_MOVE_PX = 12;
export const SHEET_MAX_WIDTH = 959;

export function canRecall(message) {
  return Boolean(message) && message.mine && !message.recalled && message.kind !== "system";
}

export function canHide(message) {
  return Boolean(message);
}

export function messageActions(message) {
  const actions = [];
  if (canRecall(message)) {
    actions.push({ id: ACTION_RECALL, label: "撤回", danger: true });
  }
  if (canHide(message)) {
    actions.push({ id: ACTION_HIDE, label: "删除", danger: true });
  }
  return actions;
}

export function confirmCopy(actionId) {
  if (actionId === ACTION_RECALL) {
    return {
      title: "撤回这条消息",
      body: "撤回后所有人都会看到「消息已撤回」，正文和图片会从服务器删除。",
      confirm: "撤回",
    };
  }
  if (actionId === ACTION_HIDE) {
    return {
      title: "删除这条消息",
      body: "只会从你这边删除，其他人仍能看到。",
      confirm: "删除",
    };
  }
  return null;
}

export function prefersFineHover(matchMedia) {
  return Boolean(matchMedia?.("(hover: hover) and (pointer: fine)")?.matches);
}

export function prefersCoarsePointer(matchMedia) {
  return Boolean(matchMedia?.("(pointer: coarse)")?.matches);
}

export function showHoverBar(hoverCapable, viewportWidth) {
  return Boolean(hoverCapable) && Number(viewportWidth) > SHEET_MAX_WIDTH;
}

export function useSheetMenu(coarsePointer, viewportWidth) {
  return Boolean(coarsePointer) || Number(viewportWidth) <= SHEET_MAX_WIDTH;
}

export function shouldLongPress(pointerType, viewportWidth) {
  if (pointerType === "touch" || pointerType === "pen") return true;
  return pointerType === "mouse" && Number(viewportWidth) <= SHEET_MAX_WIDTH;
}

export function longPressMoved(startX, startY, x, y, threshold = LONG_PRESS_MOVE_PX) {
  return Math.hypot(Number(x) - Number(startX), Number(y) - Number(startY)) > threshold;
}

export function clickOpensMenu(target) {
  return Boolean(target?.closest?.(".msg-more"));
}

export function clickIsMessageBody(target) {
  if (!target?.closest) return false;
  if (target.closest(".msg-more, .msg-hoverbar, .msg-menu, .msg-confirm, .pic")) return false;
  return Boolean(target.closest("[data-msg-id]"));
}

export function hoverBarPosition(rect, mine, viewportWidth, viewportHeight, barWidth, barHeight) {
  const vw = Number(viewportWidth);
  const vh = Number(viewportHeight);
  const width = Math.max(48, Number(barWidth) || 48);
  const height = Math.max(28, Number(barHeight) || 32);
  const top = clamp(rect.top - height + 12, 8, Math.max(8, vh - height - 8));
  const left = clamp(mine ? rect.left - 8 : rect.right - width + 8, 8, Math.max(8, vw - width - 8));
  return { top: Math.round(top), left: Math.round(left) };
}

export function menuPosition({ x, y, viewportWidth, viewportHeight, menuWidth, menuHeight, sheet }) {
  if (sheet) return { mode: "sheet" };
  const width = Math.max(120, Number(menuWidth) || 180);
  const height = Math.max(40, Number(menuHeight) || 88);
  const left = clamp(Number(x), 8, Math.max(8, Number(viewportWidth) - width - 8));
  const top = clamp(Number(y), 8, Math.max(8, Number(viewportHeight) - height - 8));
  return { mode: "menu", left: Math.round(left), top: Math.round(top) };
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}
