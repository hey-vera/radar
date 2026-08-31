// SPDX-License-Identifier: Apache-2.0
//! The signature meter: what it records, what it refuses, and what it must not
//! leak into a file that outlives the request.

use radar_customer::{Allowance, MIN_SALT_BYTES, Meter, Reading, Subject, SubjectError};

const DID: &str = "did:privy:cmthhkznr0a3u0cl86prxlb7x";

fn salt() -> Vec<u8> {
    vec![0xABu8; MIN_SALT_BYTES]
}

fn subject() -> Subject {
    Subject::derive(DID, &salt()).expect("a salted subject")
}

#[test]
fn the_durable_record_does_not_contain_the_identifier_it_was_derived_from() {
    // The property ADR 0006 turns on, asserted against the *serialised* form
    // rather than against the type — because it is the file that outlives the
    // request, and a `Debug` impl or a serde rename could put the DID back into
    // it without touching this crate's logic.
    let reading = Meter::new(subject(), Allowance::per_day(10), 20_331).reading();
    let json = serde_json::to_string(&reading).expect("a reading serialises");

    assert!(
        !json.contains(DID),
        "the DID reached the durable record: {json}"
    );
    assert!(
        !json.contains("cmthhkznr0a3u0cl86prxlb7x"),
        "the application-scoped part of the DID reached the record: {json}"
    );
}

#[test]
fn two_instances_with_different_salts_cannot_have_their_counts_joined() {
    // The reason the salt exists at all. Without it, the hash is a stable
    // function of a DID alone: anyone holding a DID could recompute the digest
    // and find that customer's row in any copy of any store.
    let other: Vec<u8> = vec![0xCDu8; MIN_SALT_BYTES];
    assert_ne!(
        Subject::derive(DID, &salt()).expect("a subject"),
        Subject::derive(DID, &other).expect("a subject"),
        "the same DID under two salts must not produce the same subject"
    );
}

#[test]
fn the_same_customer_on_the_same_instance_is_the_same_subject() {
    // The other half, and it has to hold or the meter counts one customer as
    // many and the measurement ADR 0005 wants is worthless.
    assert_eq!(subject(), subject());
    // Whitespace around an identifier is a transport artefact, not a different
    // customer.
    assert_eq!(
        Subject::derive(&format!("  {DID}  "), &salt()).expect("a subject"),
        subject()
    );
}

#[test]
fn a_missing_or_weak_salt_refuses_rather_than_recording_anything() {
    // Rule 8's direction. The tempting fallbacks here are both worse than
    // refusing: recording the raw DID is the thing the hash exists to prevent,
    // and hashing without a salt produces a value that *looks* protected while
    // being recomputable by anyone holding a DID.
    assert_eq!(Subject::derive(DID, &[]), Err(SubjectError::NoSalt));

    let short = vec![0xABu8; MIN_SALT_BYTES - 1];
    assert_eq!(
        Subject::derive(DID, &short),
        Err(SubjectError::SaltTooShort {
            given: MIN_SALT_BYTES - 1,
            needed: MIN_SALT_BYTES,
        }),
        "one byte short is short"
    );
    assert!(
        Subject::derive(DID, &[0xABu8; MIN_SALT_BYTES]).is_ok(),
        "and exactly the minimum is enough"
    );

    assert_eq!(Subject::derive("   ", &salt()), Err(SubjectError::Empty));
}

#[test]
fn an_unconfigured_allowance_refuses_every_signature() {
    // The failure that must not be available by accident. An allowance that
    // failed to load is `CLOSED`, and a signer that signs because nobody set a
    // ceiling is exactly the unbounded signer invariant 1 exists to prevent.
    let mut meter = Meter::new(subject(), Allowance::CLOSED, 20_331);
    assert_eq!(meter.charge(), Err(Allowance::CLOSED));
    assert_eq!(meter.today(), 0, "nothing was consumed");
    assert_eq!(meter.refused(), 1, "and the refusal is visible");
}

#[test]
fn the_ceiling_is_reached_exactly_and_refusals_are_counted_past_it() {
    // Swept to the boundary, because "refuses eventually" is not the property —
    // the property is that it permits exactly the allowance and not one more.
    let mut meter = Meter::new(subject(), Allowance::per_day(3), 20_331);
    for i in 1..=3 {
        assert!(
            meter.charge().is_ok(),
            "signature {i} is inside the allowance"
        );
        assert_eq!(meter.today(), i);
    }
    assert_eq!(meter.remaining(), 0);

    for i in 1..=5 {
        assert!(meter.charge().is_err(), "signature past the ceiling");
        assert_eq!(meter.today(), 3, "a refusal must not consume allowance");
        assert_eq!(
            meter.refused(),
            i,
            "a meter that is refusing must say so, not merely sit at its ceiling"
        );
    }
}

#[test]
fn a_reading_from_yesterday_does_not_spend_todays_allowance() {
    // The allowance is daily. Restoring yesterday's consumption as though it
    // were today's would refuse everything until midnight — a different bug
    // wearing this one's clothes, and a much less obvious one.
    let spent = Reading {
        subject: subject(),
        day: 20_330,
        today: 50,
        refused: 12,
    };
    let meter = Meter::restore(&spent, Allowance::per_day(10), 20_331);
    assert_eq!(meter.today(), 0, "yesterday has no claim on today");
    assert_eq!(meter.refused(), 0);
    assert_eq!(meter.remaining(), 10);
}

#[test]
fn a_reading_from_today_survives_a_restart() {
    // The half that makes it a meter rather than a counter. `radar-serve` runs
    // under `Restart=always`, and a meter that forgets on restart is not a
    // meter — a crash loop would hand out an unlimited allowance one restart at
    // a time.
    let spent = Reading {
        subject: subject(),
        day: 20_331,
        today: 8,
        refused: 2,
    };
    let mut meter = Meter::restore(&spent, Allowance::per_day(10), 20_331);
    assert_eq!(meter.today(), 8);
    assert_eq!(meter.refused(), 2);
    assert_eq!(meter.remaining(), 2);

    assert!(meter.charge().is_ok());
    assert!(meter.charge().is_ok());
    assert!(
        meter.charge().is_err(),
        "the restored consumption still counts against the ceiling"
    );
}

#[test]
fn a_reading_round_trips_through_its_serialised_form() {
    // It is the durable artefact, so the shape that goes to disk has to come
    // back. Asserted through JSON rather than through a clone, because the
    // encoding is the part that can drift.
    let original = Reading {
        subject: subject(),
        day: 20_331,
        today: 41,
        refused: 3,
    };
    let json = serde_json::to_string(&original).expect("serialises");
    let back: Reading = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, original);
}
