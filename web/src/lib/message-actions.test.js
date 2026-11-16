import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  ACTION_HIDE,
  ACTION_RECALL,
  canHide,
  canRecall,
  clickIsMessageBody,
  clickOpensMenu,
  confirmCopy,
  hoverBarPosition,
  longPressMoved,
  menuPosition,
  messageActions,
  prefersFineHover,
  shouldLongPress,
  showHoverBar,
  useSheetMenu,
} from "./message-actions.js";

function closestFactory(className, extra = {}) {
  return {
    closest(selector) {
      if (selector.split(",").some((part) => part.trim() === `.${className}` || part.trim() === className)) {
        return extra.hit || this;
      }
      if (extra.msg && selector.includes("[data-msg-id]")) return extra.msg;
      return null;
    },
  };
}

describe("message action permissions", () => {
  it("allows recall only for own live messages", () => {
    assert.equal(canRecall({ mine: true, recalled: false, kind: "text" }), true);
    assert.equal(canRecall({ mine: true, recalled: false, kind: "image" }), true);
    assert.equal(canRecall({ mine: false, recalled: false, kind: "text" }), false);
    assert.equal(canRecall({ mine: true, recalled: true, kind: "text" }), false);
    assert.equal(canRecall({ mine: true, recalled: false, kind: "system" }), false);
    assert.equal(canRecall(null), false);
  });

  it("allows hide for any visible message, matching the backend", () => {
    assert.equal(canHide({ mine: false, kind: "text" }), true);
    assert.equal(canHide({ mine: true, recalled: true, kind: "text" }), true);
    assert.equal(canHide({ kind: "system" }), true);
    assert.equal(canHide(null), false);
  });

  it("lists only permitted actions, 撤回 before 删除", () => {
    assert.deepEqual(
      messageActions({ mine: true, recalled: false, kind: "text" }).map((item) => item.id),
      [ACTION_RECALL, ACTION_HIDE],
    );
    assert.deepEqual(
      messageActions({ mine: false, recalled: false, kind: "text" }).map((item) => item.id),
      [ACTION_HIDE],
    );
    assert.deepEqual(
      messageActions({ mine: true, recalled: true, kind: "text" }).map((item) => item.id),
      [ACTION_HIDE],
    );
    assert.deepEqual(
      messageActions({ mine: true, recalled: false, kind: "system" }).map((item) => item.id),
      [ACTION_HIDE],
    );
  });

  it("keeps existing Chinese copy for destructive confirms", () => {
    const recall = confirmCopy(ACTION_RECALL);
    const hide = confirmCopy(ACTION_HIDE);
    assert.equal(recall.confirm, "撤回");
    assert.match(recall.body, /消息已撤回/);
    assert.equal(hide.confirm, "删除");
    assert.match(hide.body, /只会从你这边删除/);
    assert.equal(confirmCopy("nope"), null);
  });
});

describe("pointer rules", () => {
  it("shows the Discord-style hover bar only on wide, fine-pointer screens", () => {
    assert.equal(showHoverBar(true, 1280), true);
    assert.equal(showHoverBar(true, 959), false);
    assert.equal(showHoverBar(false, 1440), false);
    const media = (query) => ({ matches: query === "(hover: hover) and (pointer: fine)" });
    assert.equal(prefersFineHover(media), true);
    assert.equal(prefersFineHover((query) => ({ matches: query === "(pointer: coarse)" })), false);
  });

  it("uses a sheet on coarse pointers and narrow windows", () => {
    assert.equal(useSheetMenu(true, 1400), true);
    assert.equal(useSheetMenu(false, 390), true);
    assert.equal(useSheetMenu(false, 1280), false);
  });

  it("enables long-press on touch, pen, and narrow mouse windows", () => {
    assert.equal(shouldLongPress("touch", 1400), true);
    assert.equal(shouldLongPress("pen", 1400), true);
    assert.equal(shouldLongPress("mouse", 390), true);
    assert.equal(shouldLongPress("mouse", 1280), false);
  });

  it("cancels a long-press after moving past the slop", () => {
    assert.equal(longPressMoved(10, 10, 12, 11), false);
    assert.equal(longPressMoved(10, 10, 30, 10), true);
  });
});

describe("click targets", () => {
  it("opens the menu from ⋯, not from the message body", () => {
    const more = closestFactory("msg-more");
    const body = closestFactory("bubble", { msg: { dataset: { msgId: "1" } } });
    body.closest = (selector) => {
      if (selector.includes("msg-more")) return null;
      if (selector.includes("[data-msg-id]")) return { dataset: { msgId: "1" } };
      return null;
    };
    assert.equal(clickOpensMenu(more), true);
    assert.equal(clickOpensMenu(body), false);
    assert.equal(clickIsMessageBody(body), true);
    assert.equal(clickIsMessageBody(more), false);
  });
});

describe("positioning", () => {
  it("keeps the hover bar on screen", () => {
    const pos = hoverBarPosition({ top: 40, left: 900, right: 1180, bottom: 90 }, true, 1200, 800, 140, 36);
    assert.ok(pos.left >= 8);
    assert.ok(pos.left + 140 <= 1192);
    assert.ok(pos.top >= 8);
  });

  it("clamps a desktop context menu and uses a sheet when asked", () => {
    assert.deepEqual(
      menuPosition({
        x: 1100,
        y: 700,
        viewportWidth: 1200,
        viewportHeight: 800,
        menuWidth: 180,
        menuHeight: 96,
        sheet: false,
      }),
      { mode: "menu", left: 1012, top: 696 },
    );
    assert.deepEqual(
      menuPosition({
        x: 20,
        y: 20,
        viewportWidth: 390,
        viewportHeight: 800,
        menuWidth: 180,
        menuHeight: 96,
        sheet: true,
      }),
      { mode: "sheet" },
    );
  });
});
