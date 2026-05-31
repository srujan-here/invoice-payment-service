//! Id generation. We use UUIDv7 everywhere: time-ordered (so it indexes with
//! good locality, unlike v4), non-guessable (no IDOR / enumeration on public
//! ids, unlike bigserial), and mintable in-app before insert (needed to
//! correlate a payment attempt with the PSP before the row commits).

use uuid::Uuid;

/// A fresh time-ordered id.
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}
