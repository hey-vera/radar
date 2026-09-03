// SPDX-License-Identifier: Apache-2.0
//! The connect control's states, and the session that survives a reload.
//!
//! Every branch here is a thing the customer is told, and two of them are
//! claims about their wallet that must not be made when they are not true.

import { describe, expect, it, vi } from "vitest";

import { explain, storedSession } from "./Wallet";

const ADDRESS = "5Kuix3HiXh7adsdcybmr5N5coLBi2mv7exGMLvoPSKjM";

/** A storage that can be made to misbehave. */
function storage(initial?: string, throws = false) {
  return {
    getItem: vi.fn(() => {
      if (throws) throw new Error("blocked");
      return initial ?? null;
    }),
    removeItem: vi.fn(() => {
      if (throws) throw new Error("blocked");
    }),
  };
}

describe("storedSession", () => {
  it("returns a session written by a previous visit", () => {
    const session = { token: "a.b", address: ADDRESS, expiresInSeconds: 43200 };
    expect(storedSession(storage(JSON.stringify(session)))).toEqual(session);
  });

  it("returns null when nothing was stored", () => {
    expect(storedSession(storage())).toBeNull();
  });

  it("discards anything unreadable rather than repairing it", () => {
    // A half-parsed session would be sent as a bearer token and refused by the
    // server, which is a confusing way to learn that local storage was corrupt.
    for (const bad of ["not json", "null", "[]", '{"token":1}', '{"address":"x"}']) {
      const store = storage(bad);
      expect(storedSession(store)).toBeNull();
    }
  });

  it("removes a corrupt entry so it cannot be read again", () => {
    const store = storage("not json");
    storedSession(store);
    expect(store.removeItem).toHaveBeenCalled();
  });

  it("survives storage that throws outright", () => {
    // A private window, or a browser set to block site data. That is a customer
    // who signs in each time, not an error.
    expect(storedSession(storage(undefined, true))).toBeNull();
  });
});

describe("explain", () => {
  it("does not call a cancelled sign-in a failure", () => {
    // Declining to connect is a normal thing to do.
    const text = explain({ kind: "declined" });
    expect(text).toBe("Sign-in cancelled.");
    expect(text.toLowerCase()).not.toContain("error");
    expect(text.toLowerCase()).not.toContain("failed");
  });

  it("never says the wallet was rejected when the server was unreachable", () => {
    // The distinction that matters most in this file. Nothing was judged, so
    // saying the wallet was rejected is a lie about their wallet.
    const text = explain({ kind: "unreachable", detail: "Failed to fetch" });
    expect(text).toContain("not rejected");
    expect(text).toContain("Could not reach Radar");
  });

  it("passes the server's own reason through when it refuses", () => {
    // The server says why -- stale challenge, bad signature, unknown nonce --
    // and those need different remedies. Replacing them with one generic
    // message would make them one undiagnosable failure.
    expect(
      explain({
        kind: "refused",
        status: 401,
        detail: "that challenge is unknown, spent, or expired",
      }),
    ).toBe("that challenge is unknown, spent, or expired");
  });

  it("tells a first-time visitor what to install", () => {
    const text = explain({ kind: "no-wallet" });
    expect(text).toContain("Phantom");
    expect(text).toContain("Solflare");
  });

  it("renders every kind distinctly", () => {
    const texts = [
      explain({ kind: "no-wallet" }),
      explain({ kind: "declined" }),
      explain({ kind: "refused", status: 401, detail: "nope" }),
      explain({ kind: "unreachable", detail: "down" }),
    ];
    expect(new Set(texts).size).toBe(texts.length);
  });
});
