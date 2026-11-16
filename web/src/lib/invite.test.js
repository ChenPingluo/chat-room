import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  INVITE_LIMIT,
  capExplanation,
  invitedLabel,
  pendingInvites,
  quotaSummary,
} from "./invite.js";

describe("invite quota copy", () => {
  it("shows remaining slots as 已邀请 n/5", () => {
    assert.equal(INVITE_LIMIT, 5);
    assert.equal(invitedLabel({ used: 2, limit: 5, reserved: 1, remaining: 2 }), "已邀请 2/5");
    assert.equal(quotaSummary({ used: 2, reserved: 1, remaining: 2 }).atCap, false);
  });

  it("explains the cap without looking empty", () => {
    assert.match(capExplanation({ used: 5, reserved: 0, remaining: 0 }), /邀请名额已用完/);
    assert.match(capExplanation({ used: 4, reserved: 1, remaining: 0 }), /未使用的邀请码/);
    assert.equal(capExplanation({ used: 1, reserved: 0, remaining: 4 }), "");
  });

  it("lists only unused, unrevoked, unexpired codes as pending", () => {
    const now = 1_000;
    const rows = pendingInvites(
      [
        { id: 1, used_by: null, revoked: false, expires_at: 2_000 },
        { id: 2, used_by: "bob", revoked: false, expires_at: 2_000 },
        { id: 3, used_by: null, revoked: true, expires_at: 2_000 },
        { id: 4, used_by: null, revoked: false, expires_at: 500 },
      ],
      now,
    );
    assert.deepEqual(
      rows.map((row) => row.id),
      [1],
    );
  });
});
