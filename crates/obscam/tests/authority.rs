use std::{
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

use obscam::{AuthorityGate, AuthorityRejection, AuthorityState};

#[test]
fn every_takeover_gets_a_new_generation_and_fences_the_previous_holder() {
    let gate = AuthorityGate::new();
    let now = Instant::now();
    let first = gate.take(now);
    let second = gate.take(now);

    assert!(second.generation() > first.generation());
    assert_ne!(second.secret(), first.secret());
    assert_eq!(first.secret().len(), 64);
    assert_eq!(
        gate.accept(&first.credentials(), now, || "must not run"),
        Err(AuthorityRejection::NotHolder)
    );
    assert_eq!(
        gate.accept(&second.credentials(), now, || "executed"),
        Ok("executed")
    );
}

#[test]
fn lease_is_valid_before_but_not_at_the_exact_deadline() {
    let gate = AuthorityGate::new();
    let granted_at = Instant::now();
    let grant = gate.take(granted_at);

    assert_eq!(
        gate.accept(
            &grant.credentials(),
            granted_at + Duration::from_millis(4_999),
            || "executed"
        ),
        Ok("executed")
    );
    assert_eq!(
        gate.accept(
            &grant.credentials(),
            granted_at + Duration::from_secs(5),
            || "must not run"
        ),
        Err(AuthorityRejection::Expired)
    );
    assert_eq!(
        gate.snapshot_at(granted_at + Duration::from_secs(5))
            .state(),
        AuthorityState::Unheld
    );
}

#[test]
fn renewal_requires_both_current_generation_and_secret() {
    let gate = AuthorityGate::new();
    let now = Instant::now();
    let grant = gate.take(now);
    let copied_with_wrong_secret = grant.credentials().with_secret("00".repeat(32));

    assert_eq!(
        gate.renew(&copied_with_wrong_secret, now + Duration::from_secs(2)),
        Err(AuthorityRejection::NotHolder)
    );
    let renewed = gate
        .renew(&grant.credentials(), now + Duration::from_secs(2))
        .expect("current holder renews");
    assert_eq!(renewed.generation(), grant.generation());
    assert_eq!(renewed.deadline(), now + Duration::from_secs(7));
}

#[test]
fn release_and_backend_revocation_invalidate_reconnect_credentials() {
    let gate = AuthorityGate::new();
    let now = Instant::now();
    let released = gate.take(now);
    gate.release(&released.credentials(), now)
        .expect("holder releases");
    assert_eq!(
        gate.resume(&released.credentials(), now),
        Err(AuthorityRejection::NotHolder)
    );

    let revoked = gate.take(now);
    gate.revoke();
    assert_eq!(
        gate.resume(&revoked.credentials(), now),
        Err(AuthorityRejection::NotHolder)
    );
}

#[test]
fn simultaneous_takeovers_leave_exactly_one_current_holder() {
    let gate = Arc::new(AuthorityGate::new());
    let barrier = Arc::new(Barrier::new(3));
    let now = Instant::now();
    let handles = (0..2)
        .map(|_| {
            let gate = Arc::clone(&gate);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                gate.take(now)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let grants = handles
        .into_iter()
        .map(|handle| handle.join().expect("takeover thread"))
        .collect::<Vec<_>>();

    let accepted = grants
        .iter()
        .filter(|grant| gate.resume(&grant.credentials(), now).is_ok())
        .count();
    assert_eq!(accepted, 1);
}

#[test]
fn accepted_command_may_finish_without_blocking_immediate_takeover() {
    let gate = Arc::new(AuthorityGate::new());
    let now = Instant::now();
    let first = gate.take(now);
    let (work_tx, work_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let command = thread::spawn(move || {
        work_rx.recv().expect("accepted work");
        started_tx.send(()).expect("report work start");
        finish_rx.recv().expect("finish accepted command");
    });
    gate.accept(&first.credentials(), now, || {
        work_tx.send(()).expect("enqueue accepted work");
    })
    .expect("current command accepted");
    started_rx.recv().expect("accepted command started");

    let second = gate.take(now);

    finish_tx.send(()).expect("finish command");
    command.join().expect("command thread");
    assert!(second.generation() > 1);
}
