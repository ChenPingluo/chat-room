import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  canonicalizeLocation,
  guestView,
  loginHref,
  loginUrl,
  parseRoute,
  targetPath,
} from "./route.js";

function loc({ pathname = "/", hash = "", search = "", origin = "https://chat.example" } = {}) {
  return { pathname, hash, search, origin };
}

describe("targetPath", () => {
  it("opens the site URL as login", () => {
    assert.equal(targetPath(loc()), "/login");
    assert.equal(targetPath(loc({ pathname: "/", hash: "" })), "/login");
    assert.equal(targetPath(loc({ pathname: "/", hash: "#" })), "/login");
  });

  it("treats /login and /#/login as the same page", () => {
    assert.equal(targetPath(loc({ pathname: "/login" })), "/login");
    assert.equal(targetPath(loc({ hash: "#/login" })), "/login");
  });

  it("keeps invite and app routes", () => {
    assert.equal(targetPath(loc({ hash: "#/register/ab%20c" })), "/register/ab%20c");
    assert.equal(targetPath(loc({ pathname: "/register/token" })), "/register/token");
    assert.equal(targetPath(loc({ hash: "#/chat/12" })), "/chat/12");
    assert.equal(targetPath(loc({ hash: "#/friends" })), "/friends");
    assert.equal(targetPath(loc({ hash: "#/settings" })), "/settings");
    assert.equal(targetPath(loc({ hash: "#/chats" })), "/chats");
  });
});

describe("parseRoute", () => {
  it("maps unauthenticated entry to login", () => {
    assert.deepEqual(parseRoute(loc()), { name: "login" });
    assert.deepEqual(parseRoute(loc({ hash: "#/login" })), { name: "login" });
  });

  it("decodes invite tokens", () => {
    assert.deepEqual(parseRoute(loc({ hash: "#/register/a%2Fb" })), { name: "register", token: "a/b" });
  });

  it("parses chat ids", () => {
    assert.deepEqual(parseRoute(loc({ hash: "#/chat/9" })), { name: "chat", id: 9 });
  });
});

describe("guestView", () => {
  it("always shows a real form for logged-out users", () => {
    assert.equal(guestView(parseRoute(loc())), "login");
    assert.equal(guestView(parseRoute(loc({ hash: "#/chats" }))), "login");
    assert.equal(guestView(parseRoute(loc({ hash: "#/settings" }))), "login");
    assert.equal(guestView(parseRoute(loc({ hash: "#/chat/1" }))), "login");
    assert.equal(guestView(parseRoute(loc({ hash: "#/register/x" }))), "register");
  });
});

describe("canonicalizeLocation", () => {
  it("rewrites /login to a hash URL", () => {
    const calls = [];
    canonicalizeLocation(loc({ pathname: "/login", search: "?x=1" }), (...args) => calls.push(args));
    assert.deepEqual(calls, [[null, "", "/?x=1#/login"]]);
  });

  it("rewrites the bare domain to #/login", () => {
    const calls = [];
    canonicalizeLocation(loc(), (...args) => calls.push(args));
    assert.deepEqual(calls, [[null, "", "/#/login"]]);
  });

  it("leaves app hashes alone", () => {
    const calls = [];
    canonicalizeLocation(loc({ hash: "#/chats" }), (...args) => calls.push(args));
    assert.equal(calls.length, 0);
  });
});

describe("login helpers", () => {
  it("builds the public login address", () => {
    assert.equal(loginHref(loc()), "https://chat.example/#/login");
  });

  it("forces the hash to #/login after logout", () => {
    const calls = [];
    loginUrl(loc({ hash: "#/settings" }), (...args) => calls.push(args));
    assert.deepEqual(calls, [[null, "", "/#/login"]]);
  });
});
