// SPDX-License-Identifier: Apache-2.0
//! Signing in with a wallet the customer already owns.
//!
//! ADR 0011's amendment: bring-your-own-wallet first. The customer connects
//! Phantom or Solflare, signs one message to prove ownership, and gets a session
//! token back. No embedded wallet, no vendor, no custody question.
//!
//! # No wallet library
//!
//! `@solana/wallet-adapter` is the conventional choice and it is tens of
//! kilobytes before it does anything. The interface has a 120 kB gzipped budget
//! for the whole entry bundle, and the part of that library actually needed here
//! is two method calls on an object the browser already has.
//!
//! So this talks to the injected provider directly. The cost is that new wallets
//! have to be added by hand; the benefit is that the budget survives and there
//! is no third-party code between a customer and their key.
//!
//! # The message is the server's, not ours
//!
//! `/v1/customer/siws/challenge` returns the **exact text** to sign. This module
//! never assembles it. Two renderings of one message can drift, and the drift
//! surfaces as a signature that will not verify — which reads like a broken
//! wallet rather than a mismatch, and would be debugged in the wrong place.

/** The shape of an injected Solana wallet, narrowed to what is used. */
export interface WalletProvider {
  /** Phantom sets this; other wallets may not. Not relied on for anything. */
  isPhantom?: boolean;
  connect(): Promise<{ publicKey: { toString(): string } }>;
  signMessage(
    message: Uint8Array,
    encoding?: string,
  ): Promise<{ signature: Uint8Array }>;
}

/** Why a sign-in did not complete. */
export type SignInError =
  | { kind: "no-wallet" }
  | { kind: "declined" }
  | { kind: "refused"; status: number; detail: string }
  | { kind: "unreachable"; detail: string };

/** A session the server issued. */
export interface WalletSession {
  token: string;
  address: string;
  expiresInSeconds: number;
}

/** Where a browser puts an injected wallet. */
export interface WalletWindow {
  phantom?: { solana?: WalletProvider };
  solana?: WalletProvider;
}

/**
 * The injected provider, or `null` when no wallet is installed.
 *
 * Checked rather than assumed: a missing wallet is the ordinary case for a
 * first-time visitor, and it needs to render as an install prompt rather than
 * as an error.
 */
export function detect(
  // `window` carries none of these in its type, so it is widened rather than
  // cast: a cast would also silence a genuine mistake in the shape above.
  win: WalletWindow = window as unknown as WalletWindow,
): WalletProvider | null {
  // `window.phantom.solana` is the namespaced form and is preferred. The bare
  // `window.solana` is the legacy one, still set by several wallets.
  return win.phantom?.solana ?? win.solana ?? null;
}

/** What the server said, or a transport failure. */
async function post(
  fetchImpl: typeof fetch,
  path: string,
  body: unknown,
): Promise<{ ok: true; json: unknown } | { ok: false; error: SignInError }> {
  let response: Response;
  try {
    response = await fetchImpl(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (cause) {
    // A network failure is not a refusal, and conflating them would tell a
    // customer their wallet was rejected when the server was simply unreachable.
    return {
      ok: false,
      error: { kind: "unreachable", detail: String(cause) },
    };
  }
  const text = await response.text();
  if (!response.ok) {
    let detail = text;
    try {
      const parsed: unknown = JSON.parse(text);
      if (parsed && typeof parsed === "object" && "error" in parsed) {
        detail = String((parsed as { error: unknown }).error);
      }
    } catch {
      // Not JSON. The raw body is still the most useful thing to show.
    }
    return {
      ok: false,
      error: { kind: "refused", status: response.status, detail },
    };
  }
  return { ok: true, json: JSON.parse(text) as unknown };
}

/** Base64, from bytes, without pulling in a library. */
function base64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

/**
 * Connects a wallet, signs the server's challenge, and returns a session.
 *
 * The provider and `fetch` are arguments so the whole flow is testable without
 * a browser or a wallet extension — which matters, because the failure paths
 * are the ones worth testing and they are the hardest to produce by hand.
 */
export async function signIn(
  provider: WalletProvider,
  fetchImpl: typeof fetch = fetch,
): Promise<{ ok: true; session: WalletSession } | { ok: false; error: SignInError }> {
  let address: string;
  try {
    const { publicKey } = await provider.connect();
    address = publicKey.toString();
  } catch {
    // The customer closed the wallet popup. Not an error to report loudly —
    // declining to connect is a normal thing to do.
    return { ok: false, error: { kind: "declined" } };
  }

  const challenge = await post(fetchImpl, "/v1/customer/siws/challenge", {
    address,
  });
  if (!challenge.ok) return challenge;
  const { message } = challenge.json as { message: string };

  let signature: Uint8Array;
  try {
    // The server's text, byte for byte. Never reconstructed here.
    const signed = await provider.signMessage(
      new TextEncoder().encode(message),
      "utf8",
    );
    signature = signed.signature;
  } catch {
    return { ok: false, error: { kind: "declined" } };
  }

  const verified = await post(fetchImpl, "/v1/customer/siws/verify", {
    address,
    message,
    signature: base64(signature),
  });
  if (!verified.ok) return verified;

  const body = verified.json as {
    token: string;
    address: string;
    expires_in_seconds: number;
  };
  return {
    ok: true,
    session: {
      token: body.token,
      address: body.address,
      expiresInSeconds: body.expires_in_seconds,
    },
  };
}

/** A wallet address, shortened for display without losing its ends. */
export function shorten(address: string): string {
  return address.length <= 12
    ? address
    : `${address.slice(0, 4)}…${address.slice(-4)}`;
}
