// SPDX-License-Identifier: Apache-2.0
//! The connect control, and the session it produces.
//!
//! ADR 0011's amendment: the customer brings a wallet they already own, proves
//! it once, and gets a session. This is the visible half of that.
//!
//! # Every state renders as itself
//!
//! There are five, and collapsing any two of them tells the customer something
//! untrue:
//!
//! - **no wallet installed** — the ordinary case for a first-time visitor. An
//!   install prompt, not an error.
//! - **signed out** — a button.
//! - **connecting** — the wallet's popup is open and the page is waiting.
//! - **signed in** — the address, and a way out.
//! - **refused** — the server judged and said no, with its reason.
//!
//! A sixth, *unreachable*, is deliberately not folded into *refused*: one means
//! the wallet was rejected, the other that nothing was judged at all.
//!
//! # The session outlives a reload and nothing else
//!
//! Kept in `localStorage` because a customer who refreshes should not have to
//! sign again. Not a cookie: this is read by JavaScript to set a header, never
//! sent automatically, so there is no cross-site request that carries it and
//! nothing to defend against with `SameSite`.
//!
//! It is a bearer token, so anything that can read the page's storage can use
//! it — which is exactly as true of a cookie without `HttpOnly`, and the same
//! is true of the wallet extension sitting alongside it.

import { useCallback, useEffect, useState } from "react";

import {
  detect,
  shorten,
  signIn,
  type SignInError,
  type WalletSession,
} from "./siws";

/** Where the session lives between reloads. */
const STORAGE_KEY = "radar.wallet.session";

/** What the control is currently showing. */
type State =
  | { kind: "signed-out" }
  | { kind: "connecting" }
  | { kind: "signed-in"; session: WalletSession }
  | { kind: "failed"; error: SignInError };

/**
 * The stored session, if there is a usable one.
 *
 * Anything unreadable is discarded rather than repaired. A half-parsed session
 * would be sent as a bearer token and refused by the server, which is a
 * confusing way to learn that local storage was corrupt.
 */
export function storedSession(
  store: Pick<Storage, "getItem" | "removeItem"> = localStorage,
): WalletSession | null {
  let raw: string | null;
  try {
    raw = store.getItem(STORAGE_KEY);
  } catch {
    // Storage can throw outright — a private window, or a browser set to block
    // site data. That is a customer who has to sign in each time, not an error.
    return null;
  }
  if (raw === null) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      parsed !== null &&
      typeof parsed === "object" &&
      typeof (parsed as WalletSession).token === "string" &&
      typeof (parsed as WalletSession).address === "string"
    ) {
      return parsed as WalletSession;
    }
  } catch {
    // Not JSON.
  }
  try {
    store.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to do; the read already failed safely.
  }
  return null;
}

/** What to tell the customer about a failure. */
export function explain(error: SignInError): string {
  switch (error.kind) {
    case "no-wallet":
      return "No Solana wallet found. Install Phantom or Solflare to sign in.";
    case "declined":
      // Not phrased as a failure, because it is not one.
      return "Sign-in cancelled.";
    case "refused":
      return error.detail;
    case "unreachable":
      // Deliberately not "your wallet was rejected". Nothing was judged.
      return "Could not reach Radar. Your wallet was not rejected — try again.";
  }
}

/** The connect control. */
export function Wallet() {
  const [provider] = useState(() => detect());
  const [state, setState] = useState<State>(() => {
    const session = storedSession();
    return session ? { kind: "signed-in", session } : { kind: "signed-out" };
  });

  useEffect(() => {
    if (state.kind !== "signed-in") return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state.session));
    } catch {
      // Unstorable. The session still works for this page's lifetime, which is
      // better than refusing to sign in because it cannot be remembered.
    }
  }, [state]);

  const connect = useCallback(async () => {
    if (!provider) {
      setState({ kind: "failed", error: { kind: "no-wallet" } });
      return;
    }
    setState({ kind: "connecting" });
    const result = await signIn(provider);
    setState(
      result.ok
        ? { kind: "signed-in", session: result.session }
        : { kind: "failed", error: result.error },
    );
  }, [provider]);

  const signOut = useCallback(() => {
    try {
      localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Already unreadable, so already gone as far as this page is concerned.
    }
    setState({ kind: "signed-out" });
  }, []);

  if (state.kind === "signed-in") {
    return (
      <div className="flex items-center gap-3 text-sm">
        <span
          className="font-mono text-[var(--color-dim)]"
          title={state.session.address}
        >
          {shorten(state.session.address)}
        </span>
        <button
          type="button"
          onClick={signOut}
          className="text-[var(--color-dim)] underline hover:text-[var(--color-ink)] focus-visible:outline focus-visible:outline-2"
        >
          Sign out
        </button>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-3 text-sm">
      <button
        type="button"
        onClick={() => void connect()}
        disabled={state.kind === "connecting"}
        className="rounded border border-[var(--color-line)] px-3 py-1 hover:border-[var(--color-ink)] disabled:opacity-60 focus-visible:outline focus-visible:outline-2"
      >
        {state.kind === "connecting" ? "Check your wallet…" : "Connect wallet"}
      </button>
      {state.kind === "failed" && (
        <span role="status" className="text-[var(--color-refuse)]">
          {explain(state.error)}
        </span>
      )}
      {!provider && state.kind === "signed-out" && (
        <span className="text-[var(--color-dim)]">
          No wallet detected.
        </span>
      )}
    </div>
  );
}
