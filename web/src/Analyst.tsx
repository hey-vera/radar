// SPDX-License-Identifier: Apache-2.0
//! What the public analyst said, beside the evidence it said it from.
//!
//! # The page's whole reason
//!
//! Before the account is public, somebody has to read a few hundred replies and
//! disagree with some of them. That is the last thing anybody would naturally
//! do and the cheapest thing to do first, and it needs the reply *next to* the
//! fact sheet it came from — a reply on its own is a sentence to have an opinion
//! about, and a reply beside its evidence is a claim that can be checked.
//!
//! Afterwards it is the operator's window on a live account: what was asked,
//! what was answered, and the gap between what was answered and what was
//! actually published.
//!
//! # Two numbers, not one
//!
//! **Answered** and **published** are different, and the difference is the thing
//! worth watching. A publisher that is down all night fills the log and tells
//! nobody anything, and a page reporting one number would show a busy account.

import { useEffect, useState } from "react";
import { ApiError, operator, type Replies, type Reply } from "./api";

/** A time, in the reader's own zone. */
function when(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
}

/** One reply, with the evidence behind it. */
function Row({ reply }: { reply: Reply }) {
  const [open, setOpen] = useState(false);
  return (
    <li className="border-b border-slate-800 py-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
        <span className="text-slate-400">{when(reply.at)}</span>
        <span className="text-slate-500">from {reply.summoner}</span>
        {reply.mint && (
          <span className="font-mono text-xs text-slate-500">{reply.mint}</span>
        )}
        {/* Published and merely decided are visually distinct, because the
            distinction is the point of the log. */}
        {reply.reply_id ? (
          <span className="rounded bg-emerald-900/50 px-1.5 py-0.5 text-xs text-emerald-300">
            published
          </span>
        ) : (
          <span className="rounded bg-slate-800 px-1.5 py-0.5 text-xs text-slate-400">
            not published
          </span>
        )}
        {/* The early warning that the voice pass is drifting. Never hidden: a
            fallback that is invisible is a fallback nobody investigates. */}
        {reply.fellback && (
          <span className="rounded bg-amber-900/40 px-1.5 py-0.5 text-xs text-amber-300">
            template — {reply.fellback}
          </span>
        )}
      </div>

      <p className="mt-2 whitespace-pre-wrap text-slate-100">{reply.reply}</p>

      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="mt-2 text-xs text-slate-400 underline hover:text-slate-200"
      >
        {open ? "hide the evidence" : "show the evidence"}
      </button>
      {open && (
        <pre className="mt-2 overflow-x-auto rounded bg-slate-900 p-3 text-xs text-slate-300">
          {reply.fact_sheet || "(no fact sheet recorded)"}
        </pre>
      )}
    </li>
  );
}

export function Analyst() {
  const [data, setData] = useState<Replies | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    operator
      .replies(controller.signal)
      .then(setData)
      .catch((e: unknown) => {
        if (!controller.signal.aborted) {
          // Defaulted, because a failure with an empty message is still a
          // failure. An earlier version stored it as-is and tested the state
          // for truthiness, so a 500 with no body rendered as "reading…"
          // forever -- a page stuck pending is indistinguishable from a slow
          // one, which is rule 9 in the interface.
          const detail = e instanceof ApiError ? e.detail : String(e);
          setError(detail || "the server did not say why");
        }
      });
    return () => controller.abort();
  }, []);

  // `!== null`, not truthiness: see the comment where this is set.
  if (error !== null) {
    // Said rather than swallowed. A page that cannot read the log must not look
    // like a page reporting an account that said nothing.
    return (
      <section>
        <h1 className="text-xl text-slate-100">Analyst</h1>
        <p className="mt-3 text-amber-300">cannot read the reply log: {error}</p>
      </section>
    );
  }

  if (!data) {
    return (
      <section>
        <h1 className="text-xl text-slate-100">Analyst</h1>
        <p className="mt-3 text-slate-400">reading…</p>
      </section>
    );
  }

  return (
    <section>
      <h1 className="text-xl text-slate-100">Analyst</h1>

      {!data.running ? (
        // An ordinary state, not a failure: no log means the daemon has not run
        // here. Said plainly, and never as reassurance.
        <p className="mt-3 text-slate-400">
          No reply log at <code className="text-slate-300">{data.log}</code>. The
          analyst has not run on this instance.
        </p>
      ) : (
        <>
          <p className="mt-3 text-sm text-slate-400">
            <strong className="text-slate-200">{data.answered}</strong> answered,{" "}
            <strong className="text-slate-200">{data.published}</strong>{" "}
            published
            {data.answered > 0 && data.published === 0 && (
              // The state worth naming rather than leaving to arithmetic: this
              // is what a dry run looks like, and it is also what a broken
              // credential looks like.
              <span className="text-slate-500">
                {" "}
                — nothing has been posted, which is either a dry run or a
                publisher that cannot reach the platform
              </span>
            )}
          </p>
          {data.replies.length === 0 ? (
            <p className="mt-3 text-slate-400">
              The log is empty. It has started and answered nothing.
            </p>
          ) : (
            <ul className="mt-4">
              {data.replies.map((reply) => (
                <Row key={`${reply.mention_id}-${reply.at}`} reply={reply} />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
