// SPDX-License-Identifier: Apache-2.0
//! The subscription path: spawn the vendor's CLI and let it own the credential.
//!
//! # Why there is no OAuth client here
//!
//! Verified against the vendor's current documentation rather than recalled:
//! `codex login --device-auth` is the documented headless flow, credentials live
//! at `auth.json` under `CODEX_HOME`, the CLI refreshes on its own when the
//! stored token is about eight days old, and the documented pattern for
//! unattended use is *run the CLI and persist the updated file* — explicitly not
//! calling refresh endpoints yourself. Refresh breaks when another machine
//! rotated first: the credential is **single-writer**.
//!
//! So a second OAuth client inside Radar would be a second writer against a
//! single-writer contract, implementing a refresh protocol owned by a vendor who
//! will change it, in a process that also renders attacker-controlled token
//! names. Spawning the CLI costs a process and buys all of that away.
//!
//! # What this is and is not
//!
//! Radar contains no code on this path that reads, writes, parses or stores a
//! credential. That is a property of Radar's source. It is *not* a claim that
//! the operating system prevents it: a CLI running as Radar's own user has a
//! credential file Radar's user can read.
//!
//! The command is configuration precisely so it need not be. Point
//! `RADAR_MODEL_CODEX` at a wrapper that drops to a separate user and the
//! boundary stops being a sentence in this file and starts being one the kernel
//! enforces. `deploy/README.md` carries the unit that does that, and this
//! distinction is written down because the tempting version of this comment
//! claims the stronger thing.

use core::time::Duration;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

use radar_types::MicroUsd;

use crate::{Answer, Provider, Request, Unreachable, non_empty};

/// What one call is charged against the day, on a subscription.
///
/// **Not a price.** A subscription has no marginal dollar cost per call, so
/// charging one is a fiction — but a meter that charges nothing is a meter that
/// never refuses, and the thing worth bounding on this path is the *rate*: a
/// chat box left open in a loop, or a caller retrying a slow call, should run
/// out of something. This is that unit, denominated in the currency the meter
/// already speaks so there is not a second accounting system.
///
/// The API-key path computes a real cost from real token counts and does not
/// use this.
pub const NOMINAL_CALL: MicroUsd = MicroUsd(10_000);

/// Environment variables the child is allowed to inherit.
///
/// Everything else is cleared. `radar-serve`'s environment holds an x402 payout
/// address, a facilitator URL and — on the other path — a model API key, and
/// none of it is any business of a subprocess whose input is partly written by
/// whoever named a token.
///
/// `CODEX_HOME` is on the list because it is how the CLI is pointed at its own
/// credential directory, and `PATH` because the command is resolved through it.
const INHERITED: &[&str] = &["PATH", "HOME", "CODEX_HOME", "LANG", "LC_ALL", "TMPDIR"];

/// The subcommand that asks a question.
///
/// The trailing `-` is what makes the CLI read the prompt from stdin. Passing it
/// as an argument instead would put token metadata — chosen by a stranger a
/// moment earlier — into `ps` output readable by every user on the box.
const ASK: &[&str] = &["exec", "-"];

/// The subcommand that starts the device-authorisation flow.
///
/// Prints a verification URL and a short code, then waits for a human to
/// complete the flow in a browser. Neither string is secret; that is the whole
/// design of device auth, and it is why Radar can render them in a web page.
const LINK: &[&str] = &["login", "--device-auth"];

/// The vendor CLI, spawned per call.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Codex {
    /// Program and arguments, already split.
    argv: Vec<String>,
    /// The environment handed to the child, already filtered.
    env: Vec<(String, String)>,
}

impl Codex {
    /// Builds from the configured command line.
    ///
    /// # Errors
    ///
    /// Returns a message naming the variable when the command is empty. There
    /// is nothing else to configure: the CLI owns the model choice, the
    /// credential and the refresh.
    pub fn from_vars(command: &str, get: &impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let argv = split(command);
        if argv.is_empty() {
            return Err("RADAR_MODEL_CODEX is set but names no command".to_owned());
        }
        let env = INHERITED
            .iter()
            .filter_map(|name| non_empty(get, name).map(|v| ((*name).to_owned(), v)))
            .collect();
        Ok(Self { argv, env })
    }

    /// The command as it will be spawned, with a subcommand appended.
    ///
    /// `RADAR_MODEL_CODEX` names the **base** command and Radar supplies the
    /// subcommand, because Radar runs two of them: `exec -` to ask a question
    /// and `login --device-auth` to link the credential. A wrapper that dropped
    /// to another user could otherwise only serve one of them, and the
    /// alternative — a second environment variable for the second subcommand —
    /// is two things to keep in step for no gain.
    ///
    /// Exposed because the two properties worth testing here — that the child
    /// inherits nothing it was not given, and that the command string is not run
    /// through a shell — are properties of this object rather than of a
    /// successful call, and asserting them by spawning would mean the tests only
    /// hold on a machine with the CLI installed.
    #[must_use]
    pub fn command(&self, subcommand: &[&str]) -> Command {
        let mut command = Command::new(&self.argv[0]);
        command.args(&self.argv[1..]);
        command.args(subcommand);
        // The whole point. Not `env_remove` of a known-bad list: a denylist is
        // wrong the moment somebody adds a variable and does not think of this
        // file.
        command.env_clear();
        for (name, value) in &self.env {
            command.env(name, value);
        }
        command
    }
}

/// What a human needs in order to finish linking the credential.
///
/// Both fields are printed by the CLI and both are meant to be shown to a
/// person — that is what device authorisation *is*. Neither is a credential, so
/// rendering them in a web page gives away nothing: the code is useless without
/// somebody who can sign in to the account, and it expires in minutes.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize)]
pub struct Linking {
    /// Where to go.
    pub verification_url: String,
    /// What to type when you get there.
    pub user_code: String,
}

/// How long to wait for the CLI to print the URL and the code.
///
/// Short: this is the CLI starting up and calling one endpoint, not the human
/// completing the flow. The human's part has no deadline here at all, because
/// the process is left running and the browser polls.
pub const LINK_PROMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Reads the verification URL and user code out of the CLI's output.
///
/// **Deliberately generic.** The exact wording belongs to a vendor who will
/// change it, so this looks for the shapes rather than for a sentence: the first
/// `https://` URL, and the first token that looks like a device code. A format
/// change then produces a refusal rather than a confidently wrong code, which is
/// the failure worth choosing between.
///
/// A device code is short, upper-case, and often hyphenated in the middle —
/// `ABCD-EFGH`, `WDJB-MJHT`. Requiring upper case is what stops this matching an
/// ordinary word in the surrounding prose.
#[must_use]
pub fn parse_link(output: &str) -> Option<Linking> {
    let url = output.split_whitespace().find_map(|word| {
        let trimmed = word.trim_matches(|c: char| !c.is_ascii_graphic() || "\"'.,)".contains(c));
        trimmed.starts_with("https://").then(|| trimmed.to_owned())
    })?;

    let code = output
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-')))
        .find(|word| is_device_code(word))?
        .to_owned();

    Some(Linking {
        verification_url: url,
        user_code: code,
    })
}

/// Whether a word has the shape of a device code.
///
/// Four to twelve characters, upper-case alphanumeric with at most one internal
/// hyphen, and at least one digit *or* at least six letters — which is what
/// separates `WDJB-MJHT` from `URL` and from `OK`.
fn is_device_code(word: &str) -> bool {
    let body: Vec<char> = word.chars().collect();
    if !(4..=12).contains(&body.len()) {
        return false;
    }
    if word.matches('-').count() > 1 || word.starts_with('-') || word.ends_with('-') {
        return false;
    }
    if !body
        .iter()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '-')
    {
        return false;
    }
    let letters = body.iter().filter(|c| c.is_ascii_uppercase()).count();
    let digits = body.iter().filter(|c| c.is_ascii_digit()).count();
    digits > 0 || letters >= 6
}

impl Codex {
    /// Starts the device-authorisation flow and returns what a human needs.
    ///
    /// The child is **returned still running**. It has to be: the CLI sits
    /// waiting for somebody to complete the flow in a browser, and killing it to
    /// return would abandon the login it just started. The caller owns the
    /// process from here — it must be polled for completion and killed if
    /// nobody finishes.
    ///
    /// # Errors
    ///
    /// Returns [`Unreachable`]. A CLI that printed something this cannot read is
    /// [`Unreachable::Unreadable`] carrying an excerpt, which is the one place in
    /// this module raw output is surfaced: at this point in the flow the CLI has
    /// no credential yet, and the two things it is printing are a URL and a code
    /// designed to be shown to a person. The route is behind the identity check
    /// besides.
    pub fn begin_link(&self) -> Result<(std::process::Child, Linking), Unreachable> {
        let mut child = self
            .command(LINK)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Unreachable::NoContact(format!("{} — {e}", self.argv[0])))?;

        // A thread, because `read` on a pipe blocks until bytes arrive: polling
        // it inline would mean a CLI that prints nothing and does not exit hangs
        // here forever, with the deadline below never reached. Killing the child
        // closes the pipe, which ends the thread.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        if let Some(pipe) = child.stdout.take() {
            let seen = std::sync::Arc::clone(&seen);
            std::thread::spawn(move || drain(pipe, &seen));
        }

        let started = std::time::Instant::now();
        loop {
            let output = seen.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(linking) = parse_link(&output) {
                // Left running on purpose: the CLI is now waiting for a human to
                // finish in a browser, and returning by killing it would abandon
                // the login it just started.
                return Ok((child, linking));
            }

            if let Ok(Some(status)) = child.try_wait() {
                return Err(Unreachable::Refused {
                    status: format!("the CLI exited with {status} before prompting"),
                });
            }

            if expired(started.elapsed(), LINK_PROMPT_TIMEOUT) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unreadable_prompt(&output));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Reads a pipe to exhaustion, appending as it goes.
///
/// A function rather than a closure so the loop can be run against an ordinary
/// reader in a test. The condition worth testing is the **end**: `read == 0` is
/// how a pipe reports EOF, and a version that kept going instead would spin at
/// full speed appending nothing for as long as the process lived.
fn drain(mut pipe: impl Read, into: &std::sync::Mutex<String>) {
    let mut buffer = [0u8; 512];
    while let Ok(read) = pipe.read(&mut buffer) {
        if read == 0 {
            break;
        }
        if let Ok(mut into) = into.lock() {
            into.push_str(&String::from_utf8_lossy(&buffer[..read]));
        }
    }
}

/// The failure for a CLI that prompted in a shape this cannot read.
///
/// Carries an excerpt, which is the one place in this module raw output is
/// surfaced. Justified narrowly: at this point in the flow the CLI holds no
/// credential, and what it is printing is a URL and a code designed to be handed
/// to a person. Truncated anyway, and the route is behind the identity check.
fn unreadable_prompt(output: &str) -> Unreachable {
    if output.trim().is_empty() {
        return Unreachable::TimedOut {
            seconds: LINK_PROMPT_TIMEOUT.as_secs(),
        };
    }
    let excerpt: String = output.chars().take(400).collect();
    Unreachable::Unreadable(format!(
        "the CLI printed no recognisable verification URL and code: {excerpt}"
    ))
}

/// Splits a command line on whitespace.
///
/// **Not a shell.** No quoting, no expansion, no substitution — the value comes
/// from an environment file, and handing an environment file to a shell turns
/// every variable in it into a code-execution surface. A command needing quoting
/// needs a wrapper script, which is also where the drop to another user belongs.
fn split(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_owned).collect()
}

impl Provider for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn estimate(&self) -> MicroUsd {
        NOMINAL_CALL
    }

    fn ask(&self, request: &Request) -> Result<Answer, Unreachable> {
        let mut child = self
            .command(ASK)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Unreachable::NoContact(format!("{} — {e}", self.argv[0])))?;

        // The prompt goes in on stdin rather than as an argument. An argument
        // is visible in `ps` to every user on the box, and this one contains
        // token metadata a moment after a stranger chose it.
        let prompt = request.render();
        child
            .stdin
            .take()
            .ok_or_else(|| Unreachable::NoContact("stdin was not piped".to_owned()))?
            .write_all(prompt.as_bytes())
            .map_err(|e| Unreachable::NoContact(format!("writing the prompt: {e}")))?;

        let deadline = Duration::from_secs(request.timeout_seconds);
        let output = wait_with_deadline(child, deadline, request.timeout_seconds)?;

        interpret(
            output.status.success(),
            &output.status.to_string(),
            output.stdout,
        )
    }
}

/// Turns a finished process into an answer or a refusal.
///
/// Takes a `bool` and bytes rather than an [`std::process::Output`], and the
/// reason is that this is the part worth testing: a mutant deleting the `!` on
/// the success check turns every failed call into a successful one carrying
/// whatever the CLI printed on its way out. Reaching that through a real spawn
/// would mean the tests only hold on a machine with the CLI installed, so the
/// decision is separated from the process and the process keeps no decisions.
fn interpret(success: bool, status: &str, stdout: Vec<u8>) -> Result<Answer, Unreachable> {
    if !success {
        // Only the status. The CLI writes credential-adjacent detail to stderr
        // on an auth failure, and this string reaches a log and an HTTP body.
        return Err(Unreachable::Refused {
            status: format!("the CLI exited with {status}"),
        });
    }

    let text = String::from_utf8(stdout)
        .map_err(|e| Unreachable::Unreadable(format!("stdout was not UTF-8: {e}")))?;
    if text.trim().is_empty() {
        return Err(Unreachable::Unreadable(
            "the CLI succeeded and said nothing".to_owned(),
        ));
    }

    Ok(Answer {
        text,
        // A subscription does not bill per call, so there is no cost to report.
        // The meter charges the estimate rather than nothing: rule 9, absent is
        // not zero.
        cost: None,
    })
}

/// Whether a call has outlived its deadline.
///
/// Separated for the same reason as [`interpret`]: the comparison is the whole
/// timeout, and a test that established it by actually waiting would be a test
/// that takes ninety seconds. Inclusive, so a deadline of zero expires at once
/// rather than never.
const fn expired(elapsed: Duration, deadline: Duration) -> bool {
    elapsed.as_nanos() >= deadline.as_nanos()
}

/// Waits for the child, killing it if it outlives the deadline.
///
/// `wait_with_output` has no timeout, so a CLI that hangs — waiting on a
/// re-authentication prompt it will never get, which is the realistic failure —
/// holds the caller forever. Killing and reporting is the behaviour that lets
/// `radar brief` go red instead of the request queue growing.
fn wait_with_deadline(
    mut child: std::process::Child,
    deadline: Duration,
    seconds: u64,
) -> Result<std::process::Output, Unreachable> {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if expired(started.elapsed(), deadline) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Unreachable::TimedOut { seconds });
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(Unreachable::NoContact(format!("waiting on the CLI: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn the_child_inherits_only_what_it_was_given() {
        // The property this module exists for. `radar-serve`'s environment
        // holds a payout address and, on the other path, a model key; a
        // subprocess reading a token name has no business with either.
        let codex = Codex::from_vars(
            "codex exec",
            &vars(&[
                ("PATH", "/usr/bin"),
                ("CODEX_HOME", "/var/lib/radar-agent/.codex"),
                ("RADAR_X402_PAY_TO", "a-payout-address"),
                ("RADAR_MODEL_API_KEY", "sk-not-a-real-key"),
                ("AWS_SECRET_ACCESS_KEY", "also-not-real"),
            ]),
        )
        .expect("a command is the whole configuration");

        let command = codex.command(ASK);
        let passed: Vec<_> = command
            .get_envs()
            .filter_map(|(k, v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_owned())))
            .collect();

        // Sorted, because `Command` keeps its overrides in a map and the order
        // it hands them back is its own business. What is being asserted is the
        // *set*: exactly the two that were both on the allowlist and present.
        let mut names: Vec<&str> = passed.iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["CODEX_HOME", "PATH"], "only the allowlist");
        let rendered = format!("{passed:?}");
        for secret in ["sk-not-a-real-key", "a-payout-address", "also-not-real"] {
            assert!(!rendered.contains(secret), "{secret} reached the child");
        }
    }

    #[test]
    fn the_allowlist_is_an_allowlist_rather_than_a_denylist() {
        // A denylist is wrong the moment somebody adds a variable and does not
        // think of this file, which is every time. Asserted by handing over a
        // variable invented for this test: nothing knows it is a secret, and it
        // still does not get through.
        let codex = Codex::from_vars(
            "codex",
            &vars(&[("PATH", "/usr/bin"), ("A_VARIABLE_ADDED_LATER", "value")]),
        )
        .expect("built");
        assert!(
            !codex
                .command(ASK)
                .get_envs()
                .any(|(k, _)| k == OsStr::new("A_VARIABLE_ADDED_LATER"))
        );
    }

    #[test]
    fn the_command_is_not_run_through_a_shell() {
        // The value comes from an environment file. Handing an environment file
        // to a shell turns every variable in it into a code-execution surface,
        // so the metacharacters stay literal and become an argument that the
        // program will simply not understand.
        let codex = Codex::from_vars("codex exec; rm -rf / #", &vars(&[])).expect("built");
        let command = codex.command(ASK);
        assert_eq!(command.get_program(), OsStr::new("codex"));
        let args: Vec<_> = command.get_args().map(OsStr::to_string_lossy).collect();
        assert_eq!(args, ["exec;", "rm", "-rf", "/", "#", "exec", "-"]);
    }

    #[test]
    fn a_command_that_is_only_whitespace_is_not_a_command() {
        // Rule 8. The tempting failure is to spawn the empty string, which on
        // some platforms is a confusing error a long way from the cause.
        let why = Codex::from_vars("   \t ", &vars(&[])).expect_err("not a command");
        assert!(why.contains("RADAR_MODEL_CODEX"), "names it: {why}");
    }

    #[test]
    fn a_missing_binary_is_reported_as_no_contact_and_names_it() {
        // The realistic first failure: the unit is installed and the CLI is
        // not. An operator needs the program name in the message, because
        // "could not reach the provider" fits both this and an expired token.
        let codex = Codex::from_vars(
            "radar-no-such-binary-eb1f3c",
            &vars(&[("PATH", "/nonexistent")]),
        )
        .expect("built");
        let failure = codex
            .ask(&Request::new("s", "q"))
            .expect_err("there is no such binary");
        let Unreachable::NoContact(why) = &failure else {
            panic!("expected NoContact, got {failure:?}");
        };
        assert!(why.contains("radar-no-such-binary-eb1f3c"), "{why}");
    }

    #[test]
    fn a_cli_that_exited_badly_is_a_refusal_however_much_it_printed() {
        // The sharp one, and it is one deleted `!` away: a CLI that failed
        // still wrote something to stdout, and reading that as an answer turns
        // an authentication failure into a confident-looking reply.
        let failure = interpret(false, "exit status: 1", b"partial output".to_vec())
            .expect_err("a non-zero exit is not an answer");
        let Unreachable::Refused { status } = &failure else {
            panic!("expected a refusal, got {failure:?}");
        };
        assert!(status.contains("exit status: 1"), "{status}");
        assert!(
            !status.contains("partial output"),
            "stdout is not echoed into the refusal: {status}"
        );
    }

    #[test]
    fn a_cli_that_succeeded_and_said_nothing_is_not_an_empty_answer() {
        // An empty reply rendered to an operator reads as "the model had
        // nothing to say", which is a different fact from "the CLI is broken".
        for quiet in [b"".to_vec(), b"   \n\t ".to_vec()] {
            assert!(
                matches!(
                    interpret(true, "exit status: 0", quiet.clone()),
                    Err(Unreachable::Unreadable(_))
                ),
                "{quiet:?} is not an answer"
            );
        }
        // And non-UTF-8 is unreadable rather than lossily rendered.
        assert!(matches!(
            interpret(true, "exit status: 0", vec![0xff, 0xfe]),
            Err(Unreachable::Unreadable(_))
        ));
    }

    #[test]
    fn a_successful_call_reports_its_text_and_no_cost() {
        let answer = interpret(
            true,
            "exit status: 0",
            b"the creator has 41 launches".to_vec(),
        )
        .expect("a successful call with output");
        assert_eq!(answer.text, "the creator has 41 launches");
        assert_eq!(answer.cost, None, "a subscription does not bill per call");
    }

    #[test]
    fn the_deadline_is_inclusive_so_a_zero_timeout_expires_at_once() {
        // Exclusive, a deadline of zero never expires and the call hangs
        // forever -- which is the failure the timeout exists to prevent,
        // reachable by configuring the timeout to nothing.
        assert!(expired(Duration::ZERO, Duration::ZERO));
        assert!(expired(Duration::from_secs(90), Duration::from_secs(90)));
        assert!(expired(Duration::from_secs(91), Duration::from_secs(90)));
        assert!(!expired(Duration::from_secs(89), Duration::from_secs(90)));
        assert!(!expired(Duration::ZERO, Duration::from_secs(90)));
    }

    #[test]
    fn the_prompt_is_read_by_shape_rather_than_by_wording() {
        // The exact sentence belongs to a vendor who will change it. These are
        // several plausible spellings of the same prompt; all must work, because
        // a parser tied to one of them breaks on a CLI upgrade and the symptom
        // is a login button that stopped working for no visible reason.
        for output in [
            "Open https://auth.openai.com/device and enter code WDJB-MJHT\n",
            "  To sign in, visit https://auth.openai.com/device\n  Code: WDJB-MJHT\n",
            "user_code: WDJB-MJHT\nverification_uri: https://auth.openai.com/device\n",
            "Go to \"https://auth.openai.com/device\", then type WDJB-MJHT.",
        ] {
            let linking = parse_link(output).unwrap_or_else(|| panic!("unread: {output:?}"));
            assert_eq!(
                linking.verification_url, "https://auth.openai.com/device",
                "from {output:?}"
            );
            assert_eq!(linking.user_code, "WDJB-MJHT", "from {output:?}");
        }
    }

    #[test]
    fn a_prompt_that_is_only_half_there_is_not_a_prompt() {
        // Half a prompt rendered in a browser is a button that looks like it
        // worked. Waiting for both is right: the CLI prints them together, and
        // a partial read is a read that came too early.
        for output in [
            "",
            "Starting device authorisation...\n",
            "Open https://auth.openai.com/device\n",
            "Code: WDJB-MJHT\n",
            "Error: could not reach the authorisation server\n",
        ] {
            assert_eq!(parse_link(output), None, "{output:?} is not a full prompt");
        }
    }

    #[test]
    fn prose_around_the_code_is_not_mistaken_for_the_code() {
        // The risk in matching by shape. Ordinary upper-case words in the
        // surrounding text -- and the scheme in the URL -- must not win.
        let output = "NOTE: OPEN THE URL BELOW\nhttps://auth.openai.com/device\nCODE IS: 7QW2-NM4P";
        let linking = parse_link(output).expect("a URL and a code are both present");
        assert_eq!(linking.user_code, "7QW2-NM4P");

        // A code is found even when every word around it is upper case, because
        // it is the only one carrying a digit.
        assert!(!is_device_code("NOTE"), "too short and no digit");
        assert!(!is_device_code("OPEN"), "too short and no digit");
        assert!(!is_device_code("-ABCD"), "a leading hyphen is not a code");
        // Six letters, one hyphen, right length -- so this word passes every
        // other filter and the trailing-hyphen guard is the only thing
        // rejecting it. A shorter trailing-hyphen case would be rejected by
        // the letter count instead, and would not test this at all.
        assert!(
            !is_device_code("ABCDEF-"),
            "a trailing hyphen is not a code"
        );
        assert!(
            !is_device_code("-BCDEFG"),
            "nor a leading one at that length"
        );
        assert!(!is_device_code("AB-CD-EF"), "two hyphens is not a code");
        assert!(!is_device_code("wdjb-mjht"), "lower case is prose");
        assert!(!is_device_code("ABCDEFGHIJKLMNOP"), "too long");
        assert!(is_device_code("WDJB-MJHT"), "six letters, one hyphen");
        assert!(is_device_code("7QW2NM4P"), "digits, no hyphen");
    }

    #[test]
    fn draining_a_pipe_stops_at_its_end() {
        // `read == 0` is how a pipe reports EOF. A version that kept going
        // would spin at full speed appending nothing for as long as the process
        // lived -- a busy loop that only appears against a real subprocess,
        // which is why the loop is a function and not a closure.
        let seen = std::sync::Mutex::new(String::new());
        drain(
            std::io::Cursor::new(b"Open https://x/device code WDJB-MJHT\n"),
            &seen,
        );
        let read = seen.lock().expect("not poisoned").clone();
        assert!(read.contains("WDJB-MJHT"));
        assert!(parse_link(&read).is_some(), "and it is a usable prompt");

        // An empty reader is immediately at its end, and returns rather than
        // looping.
        let empty = std::sync::Mutex::new(String::new());
        drain(std::io::Cursor::new(b""), &empty);
        assert_eq!(empty.lock().expect("not poisoned").as_str(), "");
    }

    #[test]
    fn draining_reassembles_a_prompt_split_across_reads() {
        // The realistic case, and the reason the parser is run against the
        // accumulated buffer rather than against each read: a pipe delivers
        // whatever the writer flushed, and the URL and the code routinely
        // arrive in different chunks.
        struct Chunks(Vec<&'static [u8]>);
        impl Read for Chunks {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let chunk = self.0.remove(0);
                out[..chunk.len()].copy_from_slice(chunk);
                Ok(chunk.len())
            }
        }

        let seen = std::sync::Mutex::new(String::new());
        drain(
            Chunks(vec![
                b"Open https://auth.openai.com/dev",
                b"ice\nCode: WDJB",
                b"-MJHT\n",
            ]),
            &seen,
        );
        let read = seen.lock().expect("not poisoned").clone();
        let linking = parse_link(&read).expect("reassembled across three reads");
        assert_eq!(linking.verification_url, "https://auth.openai.com/device");
        assert_eq!(linking.user_code, "WDJB-MJHT");
    }

    #[test]
    fn an_unrecognised_prompt_is_a_refusal_rather_than_a_wrong_code() {
        // The failure worth choosing. A confidently wrong code sends somebody to
        // a page that rejects it with no explanation; a refusal says the CLI's
        // output changed, and carries enough of it to see how.
        let failure = unreadable_prompt("Something entirely new from the vendor\n");
        let Unreachable::Unreadable(why) = &failure else {
            panic!("expected Unreadable, got {failure:?}");
        };
        assert!(why.contains("Something entirely new"), "{why}");

        // And silence is a timeout rather than an unreadable prompt: nothing was
        // printed, so there is nothing to have misread.
        assert!(matches!(
            unreadable_prompt("   \n"),
            Unreachable::TimedOut { .. }
        ));
    }

    #[test]
    fn the_two_subcommands_are_appended_to_the_configured_command() {
        // `RADAR_MODEL_CODEX` names the base command because Radar runs two
        // subcommands through it. A wrapper that dropped to another user could
        // otherwise only serve one of them.
        let codex = Codex::from_vars("sudo -u radar-agent codex", &vars(&[])).expect("built");

        let ask_command = codex.command(ASK);
        let ask: Vec<_> = ask_command.get_args().map(OsStr::to_string_lossy).collect();
        assert_eq!(ask, ["-u", "radar-agent", "codex", "exec", "-"]);

        let link_command = codex.command(LINK);
        let link: Vec<_> = link_command
            .get_args()
            .map(OsStr::to_string_lossy)
            .collect();
        assert_eq!(
            link,
            ["-u", "radar-agent", "codex", "login", "--device-auth"]
        );
    }

    #[test]
    fn linking_reports_a_missing_binary_rather_than_hanging() {
        let codex = Codex::from_vars(
            "radar-no-such-binary-eb1f3c",
            &vars(&[("PATH", "/nonexistent")]),
        )
        .expect("built");
        assert!(matches!(codex.begin_link(), Err(Unreachable::NoContact(_))));
    }

    #[test]
    fn a_subscription_reports_no_cost_but_is_charged_anyway() {
        // Rule 9. A call whose cost is unknown must not be free, or a chat box
        // left open in a loop runs forever.
        let codex = Codex::from_vars("codex", &vars(&[])).expect("built");
        assert_eq!(codex.estimate(), NOMINAL_CALL);
        assert!(codex.estimate().get() > 0);
        assert_eq!(codex.name(), "codex");
    }
}
