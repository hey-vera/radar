// SPDX-License-Identifier: Apache-2.0
//! Where the account got to, and how often to look again.
//!
//! Two small things the loop needs and neither of which needs a network, so
//! both are decided here and tested here rather than inside a daemon nobody can
//! run twice the same way.

use std::time::Duration;

/// How long to wait before polling again.
///
/// # The rule
///
/// Poll quickly while the account is busy and slowly while it is not. A mention
/// answered while the thread is still alive gets screenshotted; one answered
/// ten minutes later does not — and an idle account polled every minute spends
/// a read a minute to learn nothing.
///
/// So: [`BUSY`] after a poll that found something, doubling to [`IDLE`] when it
/// does not. Doubling rather than jumping straight to idle, because activity
/// arrives in bursts and the poll after a burst is the most likely to find its
/// tail.
///
/// # What this is not
///
/// It is not a rate limiter. The platform's own ceiling and the spend meter are
/// what stop this account costing money; this only decides how often to look
/// when nothing is wrong. A failing poll backs off through
/// [`backoff`](crate::x::backoff) instead, which is a different question with a
/// different answer.
#[must_use]
pub fn interval(found: usize, previous: Duration) -> Duration {
    if found > 0 {
        return BUSY;
    }
    let doubled = previous.saturating_mul(2).max(BUSY);
    doubled.min(IDLE)
}

/// How often to poll while mentions are arriving.
pub const BUSY: Duration = Duration::from_secs(60);

/// The slowest this account looks for a mention.
///
/// Five minutes. Past that the delay is long enough that the thread somebody
/// asked in has moved on, which makes the answer worthless rather than merely
/// late.
pub const IDLE: Duration = Duration::from_secs(300);

/// Reads the last answered mention's id.
///
/// Absent is `None`, not an error and not zero: a fresh instance has answered
/// nothing, and the first poll then asks for whatever the platform considers
/// recent rather than for everything that has ever mentioned the account.
///
/// A file holding something that is not an id reads as `None` too. The
/// alternative — refusing to start — turns one corrupt byte into an outage,
/// and the cost of `None` is bounded: the admission gate's per-mint dedupe is
/// what stops a re-read becoming a second reply.
///
/// # Errors
///
/// Never. The signature is infallible on purpose: every failure here has the
/// same correct answer.
#[must_use]
pub fn read_cursor(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Records the last answered mention's id, atomically.
///
/// # Why the rename
///
/// A cursor written in place can be interrupted, and a half-written one is
/// digits — so it parses, and it points somewhere arbitrary. Writing beside the
/// file and renaming means the cursor is either the old id or the new one, and
/// a rename within a directory is atomic on every platform this runs on.
///
/// The failure this prevents is not theoretical for this crate: a cursor
/// truncated to its first few digits is a much smaller number, which means the
/// next poll asks for everything since a point far in the past and the account
/// answers a day of mentions again.
///
/// # Errors
///
/// The I/O error when the file cannot be written or renamed. A caller that
/// cannot save its place should stop rather than carry on: the alternative is
/// re-answering everything after the next restart.
pub fn write_cursor(path: &str, since_id: &str) -> std::io::Result<()> {
    let temp = format!("{path}.new");
    std::fs::write(&temp, since_id)?;
    std::fs::rename(&temp, path)
}

/// The id to poll from next, given what a page contained.
///
/// The **largest** id seen, not the last in the page. The platform returns
/// newest first, so "the last one" is the oldest of the batch, and using it
/// would re-read the whole page every time — answering nothing new while paying
/// for the read.
///
/// Ids are compared by length first and then lexically, because they are
/// decimal numbers that outgrew `u64` long ago and a numeric parse would either
/// truncate or fail. Longer is larger for a decimal with no leading zeros.
#[must_use]
pub fn next_cursor<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    current: Option<&'a str>,
) -> Option<String> {
    ids.into_iter()
        .chain(current)
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| (*a).cmp(*b)))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("radar-poll-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let p = dir.join(name);
        let p = p.to_str().expect("a path").to_owned();
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn a_busy_account_is_polled_quickly_again() {
        assert_eq!(interval(1, IDLE), BUSY);
        assert_eq!(interval(9, Duration::from_secs(180)), BUSY);
    }

    #[test]
    fn an_idle_account_doubles_towards_the_ceiling_rather_than_jumping() {
        // Doubling rather than jumping, because activity arrives in bursts and
        // the poll after a burst is the most likely to find its tail.
        assert_eq!(interval(0, BUSY), Duration::from_secs(120));
        assert_eq!(
            interval(0, Duration::from_secs(120)),
            Duration::from_secs(240)
        );
        assert_eq!(interval(0, Duration::from_secs(240)), IDLE);
    }

    #[test]
    fn the_wait_never_exceeds_the_ceiling_or_falls_below_the_floor() {
        assert_eq!(interval(0, IDLE), IDLE);
        assert_eq!(interval(0, Duration::from_secs(86_400)), IDLE);
        // A zero or absurdly small previous interval must not produce a busy
        // loop that polls continuously and spends a read every few
        // milliseconds.
        assert_eq!(interval(0, Duration::ZERO), BUSY);
        assert_eq!(interval(0, Duration::from_millis(1)), BUSY);
        assert_eq!(interval(0, Duration::MAX), IDLE);
    }

    #[test]
    fn a_missing_cursor_is_none_rather_than_an_error() {
        assert_eq!(read_cursor(&temp("absent")), None);
    }

    #[test]
    fn a_cursor_round_trips() {
        let path = temp("cursor");
        write_cursor(&path, "1789000000000000001").expect("written");
        assert_eq!(read_cursor(&path).as_deref(), Some("1789000000000000001"));
    }

    #[test]
    fn a_cursor_that_is_not_an_id_reads_as_none_rather_than_stopping_the_bot() {
        // One corrupt byte must not become an outage. The cost of `None` is
        // bounded by the admission gate's per-mint dedupe.
        for junk in ["", "   ", "not-an-id", "17e9", "../../etc/passwd", "-5"] {
            let path = temp("junk");
            std::fs::write(&path, junk).expect("written");
            assert_eq!(read_cursor(&path), None, "{junk:?} must not parse");
        }
    }

    #[test]
    fn a_cursor_with_surrounding_whitespace_still_reads() {
        // Somebody will edit this file by hand during an incident, and an
        // editor that adds a trailing newline must not reset the account to the
        // beginning of time.
        let path = temp("spaced");
        std::fs::write(&path, "  1789  \n").expect("written");
        assert_eq!(read_cursor(&path).as_deref(), Some("1789"));
    }

    #[test]
    fn writing_a_cursor_leaves_no_temporary_file_behind() {
        let path = temp("atomic");
        write_cursor(&path, "42").expect("written");
        assert!(!std::path::Path::new(&format!("{path}.new")).exists());
        assert_eq!(read_cursor(&path).as_deref(), Some("42"));
    }

    #[test]
    fn a_second_write_replaces_the_first_rather_than_appending() {
        let path = temp("replace");
        write_cursor(&path, "100").expect("written");
        write_cursor(&path, "200").expect("written");
        assert_eq!(read_cursor(&path).as_deref(), Some("200"));
    }

    #[test]
    fn the_next_cursor_is_the_largest_id_not_the_last_one() {
        // The platform returns newest first, so "the last in the page" is the
        // oldest of the batch. Using it would re-read the whole page forever,
        // answering nothing new and paying for the read every time.
        let page = [
            "1789000000000000009",
            "1789000000000000005",
            "1789000000000000001",
        ];
        assert_eq!(
            next_cursor(page, None).as_deref(),
            Some("1789000000000000009")
        );
    }

    #[test]
    fn ids_are_compared_as_numbers_rather_than_as_text() {
        // These outgrew u64 long ago, so they are compared by length first.
        // Lexically, "9" beats "10" and the cursor would go backwards.
        assert_eq!(next_cursor(["9", "10"], None).as_deref(), Some("10"));
        assert_eq!(
            next_cursor(["1789000000000000001", "999999999999999999"], None).as_deref(),
            Some("1789000000000000001")
        );
    }

    #[test]
    fn the_cursor_never_moves_backwards_when_a_page_is_older_than_it() {
        // A page of old mentions -- a replayed request, or a platform serving
        // something stale -- must not rewind the account into re-answering.
        assert_eq!(
            next_cursor(["100", "200"], Some("500")).as_deref(),
            Some("500")
        );
    }

    #[test]
    fn an_empty_page_keeps_the_cursor_where_it_was() {
        assert_eq!(next_cursor([], Some("500")).as_deref(), Some("500"));
        assert_eq!(next_cursor([], None), None);
    }

    #[test]
    fn a_malformed_id_in_a_page_cannot_become_the_cursor() {
        // Every id here came off the network. One that is not digits would be
        // written to the cursor file and read back as `None` next start, which
        // resets the account to the beginning of time.
        assert_eq!(
            next_cursor(["not-an-id", "", "300"], None).as_deref(),
            Some("300")
        );
        assert_eq!(next_cursor(["not-an-id"], None), None);
    }
}
