// SPDX-License-Identifier: Apache-2.0
//! The sign-in flow, driven without a browser or a wallet extension.
//!
//! The failure paths are the ones worth testing and the hardest to produce by
//! hand: a customer closing the popup, a server refusing a stale challenge, a
//! network that is simply down. All three render differently, and conflating
//! any two of them tells a customer something untrue about why they are not
//! signed in.

import { describe, expect, it, vi } from "vitest";

import { detect, shorten, signIn, type WalletProvider } from "./wallet";

const ADDRESS = "5Kuix3HiXh7adsdcybmr5N5coLBi2mv7exGMLvoPSKjM";
const MESSAGE = `radar.heyvera.org wants you to sign in with your Solana account:\n${ADDRESS}\n\nSigning proves you own this wallet. It authorises no transaction and moves no funds.\n\nNonce: abc123\nIssued At: 1788000000`;

/** A wallet that connects and signs whatever it is given. */
function wallet(overrides: Partial<WalletProvider> = {}): WalletProvider {
  return {
    connect: async () => ({ publicKey: { toString: () => ADDRESS } }),
    signMessage: async () => ({ signature: new Uint8Array([1, 2, 3]) }),
    ...overrides,
  };
}

/** A server that answers both endpoints. */
function server(
  responses: { challenge?: Response; verify?: Response } = {},
): typeof fetch {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("challenge")) {
      return (
        responses.challenge ??
        new Response(JSON.stringify({ message: MESSAGE, expires_in_seconds: 300 }), {
          status: 200,
        })
      );
    }
    return (
      responses.verify ??
      new Response(
        JSON.stringify({
          token: "payload.tag",
          address: ADDRESS,
          expires_in_seconds: 43200,
        }),
        { status: 200 },
      )
    );
  }) as unknown as typeof fetch;
}

describe("signIn", () => {
  it("returns a session when the wallet signs", async () => {
    const result = await signIn(wallet(), server());
    expect(result).toEqual({
      ok: true,
      session: { token: "payload.tag", address: ADDRESS, expiresInSeconds: 43200 },
    });
  });

  it("signs the server's exact text and never its own", async () => {
    // The property the module doc argues for. Two renderings of one message can
    // drift, and the drift surfaces as a signature that will not verify -- which
    // reads like a broken wallet and gets debugged in the wrong place.
    let signed: string | undefined;
    const provider = wallet({
      signMessage: async (bytes: Uint8Array) => {
        signed = new TextDecoder().decode(bytes);
        return { signature: new Uint8Array([9]) };
      },
    });
    await signIn(provider, server());
    expect(signed).toBe(MESSAGE);
  });

  it("sends back the same message it signed", async () => {
    // Not a reconstruction. If the body carried a rebuilt message the signature
    // would be over different bytes than the server checks.
    const fetchImpl = server();
    await signIn(wallet(), fetchImpl);
    const calls = (fetchImpl as unknown as { mock: { calls: unknown[][] } }).mock
      .calls;
    const verify = calls.find((c) => String(c[0]).includes("verify"));
    const body = JSON.parse(
      (verify?.[1] as { body: string }).body,
    ) as { message: string; address: string; signature: string };
    expect(body.message).toBe(MESSAGE);
    expect(body.address).toBe(ADDRESS);
    expect(body.signature).toBe(btoa("\x01\x02\x03"));
  });

  it("reports a closed popup as declined rather than as an error", async () => {
    // Declining to connect is a normal thing to do, and must not render as
    // something having gone wrong.
    const refusedConnect = await signIn(
      wallet({
        connect: async () => {
          throw new Error("User rejected the request.");
        },
      }),
      server(),
    );
    expect(refusedConnect).toEqual({ ok: false, error: { kind: "declined" } });

    const refusedSignature = await signIn(
      wallet({
        signMessage: async () => {
          throw new Error("User rejected the request.");
        },
      }),
      server(),
    );
    expect(refusedSignature).toEqual({ ok: false, error: { kind: "declined" } });
  });

  it("distinguishes a refusal from an unreachable server", async () => {
    // The distinction that matters. A 401 means the challenge was stale or the
    // signature was wrong; a network failure means nothing was judged at all.
    // Telling a customer their wallet was rejected when the server was down is
    // a lie about their wallet.
    const refused = await signIn(
      wallet(),
      server({
        verify: new Response(
          JSON.stringify({ error: "that challenge is unknown, spent, or expired" }),
          { status: 401 },
        ),
      }),
    );
    expect(refused).toEqual({
      ok: false,
      error: {
        kind: "refused",
        status: 401,
        detail: "that challenge is unknown, spent, or expired",
      },
    });

    const down = vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    }) as unknown as typeof fetch;
    const unreachable = await signIn(wallet(), down);
    expect(unreachable.ok).toBe(false);
    if (!unreachable.ok) expect(unreachable.error.kind).toBe("unreachable");
  });

  it("stops at the challenge when the challenge is refused", async () => {
    // No point asking a wallet to sign something the server will not accept,
    // and asking anyway shows the customer a popup that was always doomed.
    const provider = wallet({
      signMessage: vi.fn(async () => ({ signature: new Uint8Array([1]) })),
    });
    const result = await signIn(
      provider,
      server({
        challenge: new Response(
          JSON.stringify({ error: "this instance has no customer sign-in configured" }),
          { status: 503 },
        ),
      }),
    );
    expect(result.ok).toBe(false);
    expect(provider.signMessage).not.toHaveBeenCalled();
  });
});

describe("detect", () => {
  it("prefers the namespaced provider over the legacy one", () => {
    const namespaced = wallet();
    const legacy = wallet();
    expect(detect({ phantom: { solana: namespaced }, solana: legacy })).toBe(
      namespaced,
    );
  });

  it("falls back to the legacy provider", () => {
    const legacy = wallet();
    expect(detect({ solana: legacy })).toBe(legacy);
  });

  it("reports no wallet as null rather than throwing", () => {
    // The ordinary case for a first-time visitor. It has to render as an
    // install prompt, not as a failure.
    expect(detect({})).toBeNull();
    expect(detect({ phantom: {} })).toBeNull();
  });
});

describe("shorten", () => {
  it("keeps both ends, because the ends are what people check", () => {
    expect(shorten(ADDRESS)).toBe("5Kui…SKjM");
  });

  it("leaves something already short alone", () => {
    expect(shorten("abc")).toBe("abc");
    expect(shorten("123456789012")).toBe("123456789012");
  });
});
