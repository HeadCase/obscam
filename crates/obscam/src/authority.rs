use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use serde::Serialize;
use thiserror::Error;
use tokio::sync::broadcast;

pub(crate) const LEASE_DURATION_MS: u64 = 5_000;
const LEASE_DURATION: Duration = Duration::from_millis(LEASE_DURATION_MS);
const SECRET_BYTES: usize = 32;

/// The single serialized, RAM-only gate for global mutation authority.
#[derive(Clone, Debug)]
pub struct AuthorityGate {
    inner: Arc<Mutex<Inner>>,
    updates: broadcast::Sender<AuthoritySnapshot>,
}

#[derive(Debug)]
struct Inner {
    generation: u64,
    holder: Option<Holder>,
}

#[derive(Debug)]
struct Holder {
    secret: String,
    deadline: Instant,
}

/// Credentials issued to one browser tab for the current lease generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityCredentials {
    generation: u64,
    secret: String,
}

impl AuthorityCredentials {
    /// Reconstructs credentials supplied by an untrusted control client.
    #[must_use]
    pub fn new(generation: u64, secret: String) -> Self {
        Self { generation, secret }
    }

    /// Returns the server generation carried by these credentials.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the tab-scoped bearer secret.
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// Returns a copy with a replacement secret, useful at trust-boundary tests.
    #[must_use]
    pub fn with_secret(&self, secret: String) -> Self {
        Self {
            generation: self.generation,
            secret,
        }
    }
}

/// A newly issued authority lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityGrant {
    credentials: AuthorityCredentials,
    deadline: Instant,
}

impl AuthorityGrant {
    /// Returns the monotonically increasing generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.credentials.generation
    }

    /// Returns the long random tab-scoped secret.
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.credentials.secret
    }

    /// Returns a copy of the credentials for renewal or mutation authorization.
    #[must_use]
    pub fn credentials(&self) -> AuthorityCredentials {
        self.credentials.clone()
    }

    pub(crate) const fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// A successfully renewed or resumed current lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityLease {
    generation: u64,
    deadline: Instant,
}

impl AuthorityLease {
    /// Returns the unchanged current generation.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Returns the exact server-monotonic expiry deadline.
    #[must_use]
    pub const fn deadline(self) -> Instant {
        self.deadline
    }
}

/// Public ownership state; holder identity and credentials are never exposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityState {
    /// No viewer currently holds mutation authority.
    Unheld,
    /// Exactly one undisclosed viewer currently holds mutation authority.
    Held,
}

/// An identity-free snapshot suitable for broadcasting to every viewer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoritySnapshot {
    state: AuthorityState,
    generation: u64,
}

impl AuthoritySnapshot {
    /// Returns whether authority is currently held.
    #[must_use]
    pub const fn state(self) -> AuthorityState {
        self.state
    }

    /// Returns the latest generation, including after its lease ends.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Fail-closed reasons returned at the serialized authority gate.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AuthorityRejection {
    /// The presented generation and secret are not the current holder.
    #[error("credentials do not identify the current holder")]
    NotHolder,
    /// The server-monotonic deadline has been reached.
    #[error("the authority lease expired")]
    Expired,
}

impl AuthorityGate {
    /// Creates an unheld gate with a fresh runtime-local generation sequence.
    #[must_use]
    pub fn new() -> Self {
        let (updates, _) = broadcast::channel(16);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                generation: 0,
                holder: None,
            })),
            updates,
        }
    }

    /// Immediately preempts any holder and returns a fresh lease.
    ///
    /// # Panics
    ///
    /// Panics if the process exhausts all `u64` generations or the operating
    /// system cannot provide cryptographically secure randomness.
    #[must_use]
    pub fn take(&self, now: Instant) -> AuthorityGrant {
        let grant = {
            let mut inner = self.lock();
            inner.generation = inner
                .generation
                .checked_add(1)
                .expect("authority generation exhausted");
            let credentials = AuthorityCredentials {
                generation: inner.generation,
                secret: random_secret(),
            };
            let deadline = now + LEASE_DURATION;
            inner.holder = Some(Holder {
                secret: credentials.secret.clone(),
                deadline,
            });
            AuthorityGrant {
                credentials,
                deadline,
            }
        };
        self.publish_snapshot();
        grant
    }

    /// Extends the current holder's lease from server-monotonic `now`.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityRejection`] for stale, copied, or expired credentials.
    pub fn renew(
        &self,
        credentials: &AuthorityCredentials,
        now: Instant,
    ) -> Result<AuthorityLease, AuthorityRejection> {
        let mut inner = self.lock();
        let holder = validate(&mut inner, credentials, now)?;
        let deadline = now + LEASE_DURATION;
        holder.deadline = deadline;
        Ok(AuthorityLease {
            generation: credentials.generation,
            deadline,
        })
    }

    /// Revalidates same-tab reconnect credentials without extending the lease.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityRejection`] for stale, copied, or expired credentials.
    pub fn resume(
        &self,
        credentials: &AuthorityCredentials,
        now: Instant,
    ) -> Result<AuthorityLease, AuthorityRejection> {
        let mut inner = self.lock();
        let holder = validate(&mut inner, credentials, now)?;
        Ok(AuthorityLease {
            generation: credentials.generation,
            deadline: holder.deadline,
        })
    }

    /// Releases authority only when called by the current unexpired holder.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityRejection`] for stale, copied, or expired credentials.
    pub fn release(
        &self,
        credentials: &AuthorityCredentials,
        now: Instant,
    ) -> Result<(), AuthorityRejection> {
        let mut inner = self.lock();
        validate(&mut inner, credentials, now)?;
        inner.holder = None;
        drop(inner);
        self.publish_snapshot();
        Ok(())
    }

    /// Revokes authority for runtime or camera-backend recovery.
    pub fn revoke(&self) {
        let was_held = self.lock().holder.take().is_some();
        if was_held {
            self.publish_snapshot();
        }
    }

    /// Rechecks credentials and exact expiry, then atomically accepts or enqueues a mutation.
    /// `acceptance` must return as soon as the camera owner has accepted the work; the
    /// returned value may represent completion that proceeds after this gate is released.
    ///
    /// # Errors
    ///
    /// Returns [`AuthorityRejection`] without invoking `acceptance` when the credentials
    /// are stale, copied, or expired.
    pub fn accept<T>(
        &self,
        credentials: &AuthorityCredentials,
        now: Instant,
        acceptance: impl FnOnce() -> T,
    ) -> Result<T, AuthorityRejection> {
        let mut inner = self.lock();
        validate(&mut inner, credentials, now)?;
        Ok(acceptance())
    }

    /// Returns identity-free authoritative ownership, expiring it at `now` if due.
    #[must_use]
    pub fn snapshot_at(&self, now: Instant) -> AuthoritySnapshot {
        let mut inner = self.lock();
        let was_held = inner.holder.is_some();
        expire_if_due(&mut inner, now);
        let current = snapshot(&inner);
        drop(inner);
        if was_held && current.state == AuthorityState::Unheld {
            let _ = self.updates.send(current);
        }
        current
    }

    /// Returns identity-free ownership using the server monotonic clock.
    #[must_use]
    pub fn snapshot(&self) -> AuthoritySnapshot {
        self.snapshot_at(Instant::now())
    }

    pub(crate) fn expire_deadline(&self, generation: u64, deadline: Instant) -> bool {
        let mut inner = self.lock();
        let matches = inner.generation == generation
            && inner
                .holder
                .as_ref()
                .is_some_and(|holder| holder.deadline == deadline);
        if matches {
            inner.holder = None;
        }
        drop(inner);
        if matches {
            self.publish_snapshot();
        }
        matches
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<AuthoritySnapshot> {
        self.updates.subscribe()
    }

    fn publish_snapshot(&self) {
        let current = snapshot(&self.lock());
        let _ = self.updates.send(current);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for AuthorityGate {
    fn default() -> Self {
        Self::new()
    }
}

fn validate<'a>(
    inner: &'a mut Inner,
    credentials: &AuthorityCredentials,
    now: Instant,
) -> Result<&'a mut Holder, AuthorityRejection> {
    if inner.generation != credentials.generation {
        return Err(AuthorityRejection::NotHolder);
    }
    let Some(holder) = inner.holder.as_mut() else {
        return Err(AuthorityRejection::NotHolder);
    };
    if holder.secret != credentials.secret {
        return Err(AuthorityRejection::NotHolder);
    }
    if now >= holder.deadline {
        return Err(AuthorityRejection::Expired);
    }
    Ok(holder)
}

fn expire_if_due(inner: &mut Inner, now: Instant) {
    if inner
        .holder
        .as_ref()
        .is_some_and(|holder| now >= holder.deadline)
    {
        inner.holder = None;
    }
}

const fn snapshot(inner: &Inner) -> AuthoritySnapshot {
    AuthoritySnapshot {
        state: if inner.holder.is_some() {
            AuthorityState::Held
        } else {
            AuthorityState::Unheld
        },
        generation: inner.generation,
    }
}

fn random_secret() -> String {
    let mut bytes = [0_u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).expect("operating-system randomness unavailable");
    let mut secret = String::with_capacity(SECRET_BYTES * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(secret, "{byte:02x}").expect("writing to String cannot fail");
    }
    secret
}
