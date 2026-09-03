// SPDX-License-Identifier: Apache-2.0
//! The reading assistant: linking it, and asking it things.
//!
//! Two panels that only exist when a provider is configured. The server does not
//! mount `/v1/chat` or `/v1/link` otherwise, so a 404 here means "not
//! configured" rather than "broken" — and the component renders nothing at all
//! rather than a button that cannot work.
//!
//! # What a reply is allowed to be
//!
//! Text in a `<p>`. There is no branch on what the model said, nothing parsed
//! out of it, and no action it can reach. A reply is rendered with
//! `{answer.text}` — React escapes it — and never with `dangerouslySetInnerHTML`,
//! which would turn a model that read an attacker's token name into a model that
//! can write script into this page.
//!
//! An **uncited** reply is marked, because an uncited claim has the shape of a
//! fabrication and a reader has to be able to tell which they have.
//!
//! # This is an operator screen, not product
//!
//! `/v1/link` is `Audience::Operator` -- it drives a device-authorisation flow
//! that binds a *vendor credential* to this instance, which is an operator
//! action and not something a customer may ever reach. `/v1/chat` is
//! `Audience::Customer`, so the two halves of this component do not share an
//! audience, and the link panel is the half that must not ship to customers.
//!
//! Nothing enforces that yet, because nothing needs to: no customer
//! authenticator is configured, so every route falls back to the operator
//! check. The day `RADAR_PRIVY_APP_ID` is set, this component begins showing a
//! customer a panel whose every request is refused.
//!
//! Two things follow, and both belong to the screen split rather than here:
//! `Link` moves to the operator surface, and `/v1/chat` gains per-customer
//! metering -- today it reserves against one **global** daily budget with no
//! customer in the path, so the first signed-up account could spend the whole
//! day's allowance.

import { useCallback, useEffect, useState } from "react";
import { agent, ApiError, type Answered, type Progress } from "./api";

/**
 * Renders both panels, or nothing if the agent is not configured.
 *
 * `alwaysShow` is for the tab that exists to show this: a tab that renders
 * empty is worse than one that says why it is empty, because the reader cannot
 * tell a missing provider from a broken page.
 */
export function Agent({ alwaysShow = false }: { alwaysShow?: boolean }) {
  const [available, setAvailable] = useState<boolean | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    agent
      .linkStatus(controller.signal)
      .then(() => setAvailable(true))
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        // 404 is the configured answer for "no provider", so it is the one
        // status that means *absent* rather than *broken*. Anything else is a
        // fault and the panels stay up to show it.
        setAvailable(!(error instanceof ApiError && error.status === 404));
      });
    return () => controller.abort();
  }, []);

  if (available !== true) {
    if (!alwaysShow) return null;
    return (
      <p className="rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] px-4 py-3 text-sm">
        {available === null
          ? "Asking…"
          : "No model provider is configured, so there is nothing to ask. Set RADAR_MODEL_DAILY_USD and a provider; the routes do not exist until both are set."}
      </p>
    );
  }

  return (
    <section>
      <Link />
      <Chat />
    </section>
  );
}

/** The credential-linking panel. */
function Link() {
  const [progress, setProgress] = useState<Progress>({ state: "idle" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback((signal?: AbortSignal) => {
    agent
      .linkStatus(signal)
      .then(setProgress)
      .catch(() => {
        // A failed poll is not a failed flow. Leaving the last known state up
        // beats replacing a live code with an error the operator cannot act on.
      });
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    refresh(controller.signal);
    // Polled only while a flow is open. A timer that runs forever on an idle
    // page is a request every two seconds for as long as the tab is left open.
    if (progress.state !== "waiting") return () => controller.abort();
    const timer = setInterval(() => refresh(), 3000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [refresh, progress.state]);

  const begin = () => {
    setBusy(true);
    setError(null);
    agent
      .link()
      .then(setProgress)
      .catch((e: unknown) =>
        setError(e instanceof ApiError ? e.detail : String(e)),
      )
      .finally(() => setBusy(false));
  };

  return (
    <div className="mb-8 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] p-4">
      {progress.state === "waiting" ? (
        <>
          <p className="text-sm">
            Open{" "}
            <a
              className="underline decoration-[var(--color-line)] underline-offset-4"
              href={progress.verification_url}
              target="_blank"
              rel="noreferrer noopener"
            >
              {progress.verification_url}
            </a>{" "}
            and enter this code:
          </p>
          <p className="mt-3 font-mono text-2xl tracking-[0.2em] text-[var(--color-good)]">
            {progress.user_code}
          </p>
          <p className="mt-3 text-xs text-[var(--color-dim)]">
            Waiting — {progress.seconds_elapsed}s. This page checks every few
            seconds; the code expires in a few minutes.
          </p>
        </>
      ) : (
        <div className="flex items-center justify-between gap-4">
          <p className="text-sm text-[var(--color-dim)]">
            {progress.state === "linked"
              ? "Linked. The CLI holds the credential and refreshes it on its own."
              : progress.state === "failed"
                ? `The last attempt ended: ${progress.status}`
                : "Radar never stores a token — the vendor CLI does. Linking opens a code you enter in a browser."}
          </p>
          <button
            type="button"
            onClick={begin}
            disabled={busy}
            className="shrink-0 rounded-md border border-[var(--color-line)] px-3 py-1.5 text-sm hover:border-[var(--color-dim)] disabled:opacity-50"
          >
            {busy ? "Starting…" : progress.state === "linked" ? "Re-link" : "Link"}
          </button>
        </div>
      )}
      {error && (
        <p className="mt-3 text-xs text-[var(--color-warn)]">{error}</p>
      )}
    </div>
  );
}

/** The question box. */
function Chat() {
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<Answered | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!question.trim() || asking) return;
    setAsking(true);
    setError(null);
    setAnswer(null);
    agent
      .ask(question)
      .then(setAnswer)
      .catch((e: unknown) =>
        setError(
          e instanceof ApiError
            ? e.status === 402
              ? "The day's model budget is spent. It resets tomorrow."
              : e.detail
            : String(e),
        ),
      )
      .finally(() => setAsking(false));
  };

  return (
    <form onSubmit={submit}>
      <label className="block text-xs text-[var(--color-dim)]" htmlFor="ask">
        Ask about what Radar recorded. It cannot see a position or a price, and
        it cannot place a trade.
      </label>
      <div className="mt-2 flex gap-2">
        <input
          id="ask"
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          placeholder="Why were so many candidates refused for capacity?"
          className="min-w-0 flex-1 rounded-md border border-[var(--color-line)] bg-[var(--color-ink)] px-3 py-2 text-sm outline-none focus:border-[var(--color-dim)]"
        />
        <button
          type="submit"
          disabled={asking || !question.trim()}
          className="shrink-0 rounded-md border border-[var(--color-line)] px-3 py-2 text-sm hover:border-[var(--color-dim)] disabled:opacity-50"
        >
          {asking ? "Asking…" : "Ask"}
        </button>
      </div>

      {error && <p className="mt-3 text-sm text-[var(--color-warn)]">{error}</p>}

      {answer && (
        <div className="mt-4 rounded-md border border-[var(--color-line)] bg-[var(--color-surface)] p-4">
          {/* Text, and only text. Never `dangerouslySetInnerHTML`: a model that
              read an attacker's token name must not be able to write script
              into this page. `white-space: pre-wrap` keeps the model's own
              line breaks without interpreting anything. */}
          <p className="whitespace-pre-wrap text-sm leading-relaxed">
            {answer.text}
          </p>
          {answer.uncited ? (
            <p className="mt-3 border-t border-[var(--color-line)] pt-3 text-xs text-[var(--color-warn)]">
              Nothing was consulted for this answer. Treat it as the model's
              recollection rather than as something Radar measured.
            </p>
          ) : (
            <p className="mt-3 border-t border-[var(--color-line)] pt-3 text-xs text-[var(--color-dim)]">
              From: {answer.citations.join(", ")}
            </p>
          )}
        </div>
      )}
    </form>
  );
}
