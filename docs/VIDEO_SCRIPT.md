# Loom Demo Script — Invoice & Payment Service (target 8 min)

The brief grades **fluency, not polish**. Sections 1–2 you can read closely; **Sections 3 & 4
must be in your own words** ("if you cannot explain it in your own words, you did not make it").
Ums and retries are fine. Don't edit.

---

## BEFORE YOU RECORD (5-min setup, camera OFF)

1. **Start the stack** and leave it running:
   ```bash
   docker compose up --build
   ```
   Wait until logs are quiet, then in another terminal: `curl -s http://localhost:8080/healthz` → `ok`.
2. **webhook.site**: open https://webhook.site, copy your unique URL.
3. **Postman**: import `docs/postman_collection.json`. In the collection **Variables** tab, paste your
   webhook.site URL into `webhook_url`. Confirm `base_url` and `api_key` are filled.
4. **Editor**: open these files in tabs, in this order, ready to show:
   - `DESIGN.md` (scroll to §2 state diagram, and §3 failure modes)
   - `crates/invoice-service/src/services/payment_service.rs`
   - `crates/invoice-service/src/psp/mod.rs`
   - `migrations/0008_payment_attempts.sql`
5. **Window layout**: editor + Postman + a browser tab on webhook.site + one terminal. Decide your
   alt-tab order now so you're not hunting on camera.
6. Do **one silent dry-run** of the Postman flow so variables are warm and you know the clicks.

Checkpoint before recording: `docker compose ps` shows db, app, mock-psp all up.

---

## SECTION 1 — Architecture overview  (~1:30)   [read OK]

**Show:** the editor file tree (`crates/`, `migrations/`), briefly.

> "Hi — this is my Invoice & Payment Service for the Dodo take-home. It's written in Rust with
> Axum, sqlx, Tokio and Postgres.
>
> It's a Cargo workspace with three crates: a shared `common` crate, the main `invoice-service`,
> and a standalone `mock-psp` that stands in for a real payment provider.
>
> Inside the service the layering is strict and one-directional: **routes** only parse input and
> map results, **services** hold the business logic and own the database transactions,
> **repositories** are sqlx-only and every query is scoped by `business_id`. Money is integer
> cents everywhere — a `Money` newtype over `i64` that deliberately has no float conversion, so
> floats are unrepresentable in the money path.
>
> The flow of a request: a business authenticates with an API key; middleware resolves it to a
> `business_id`. To pay an invoice, the service runs **two short transactions around the PSP
> call** and never holds a database transaction across that HTTP call. Every state change also
> writes an event into a **webhook outbox table in the same transaction**, and two background
> Tokio tasks — a webhook **dispatcher** and a payment **reconciler** — drain that work
> asynchronously, so webhook delivery never blocks the API response."

**Achieve:** grader understands the stack, the layering, integer money, and that webhooks +
unknown-payment handling are async/decoupled.

---

## SECTION 2 — Live demo  (~3:00)   [read OK; drive Postman]

**(a) Prove it boots.** Show the terminal running `docker compose up` and the `healthz` → `ok`.
> "Everything comes up with a single `docker compose up` — database, mock PSP, and the service,
> with migrations applied automatically. Health check is green."

**(b) Switch to Postman.** Run requests in folder order; narrate each:

1. **1. Customers → Create.**
   > "I create a customer — the response id is captured automatically into a collection variable."
2. **2. Invoices → Create.** Point at the body's `total_cents: 999999`, then the response.
   > "Notice I send a bogus total of 999999, but the server computes the real total from the line
   > items — 3 times 1500 — and returns **4500**. The client total is ignored. The test on the
   > right confirms it."
3. **2. Invoices → Finalize.**
   > "Finalize moves it from `draft` to `open` so it can be paid."
4. **3. Payments → SUCCESS (tok_success).**
   > "Paying with `tok_success` — 200, status succeeded, a `psp_ref`, and the invoice is now
   > `paid`."
5. **3. Payments → IDEMPOTENT REPLAY.**
   > "Same Idempotency-Key, same body — I get back the **identical** attempt id, the cached 200,
   > and the PSP is **not** called again. That's idempotency."
6. **3. Payments → Setup invoice 2**, then **DECLINED (tok_card_declined).**
   > "On a fresh invoice, `tok_card_declined` returns **402** with a failure code, and importantly
   > the invoice stays **open** — it's retryable, not dead."
7. **4. Webhooks → register** (already pointed at webhook.site), then **GET /v1/events** and
   **GET /v1/webhook-deliveries.**
   > "Here are the emitted events — invoice.created, invoice.paid, invoice.payment_failed — and
   > their delivery status, all succeeded."

**(c) Flip to the webhook.site browser tab.**
> "And here they are arriving live at my receiver — each one carries an `X-Webhook-Id` and an
> `X-Webhook-Signature` header. They're HMAC-SHA256 signed so the receiver can verify
> authenticity."

**Achieve:** the five required actions are visibly done (customer, invoice, success pay, declined
pay, webhook deliveries), plus the two money/idempotency proofs.

---

## SECTION 3 — State-machine walkthrough  (~1:30)   [UNSCRIPTED — your own words]

**Show:** `DESIGN.md` §2 (the Mermaid diagram). Speak from these anchors:

- States: **draft → open → paid**, plus **open → void** and **open → uncollectible** (and a draft
  can be voided).
- Why: draft is editable; **finalize** freezes the line items and opens it for payment.
- **Terminal** states: paid, void, uncollectible — nothing transitions out of them.
- **The decision you made:** "I chose **not** to add a transient `charging` state. An in-flight
  payment is a `pending` row in `payment_attempts`, and that row pins the invoice through a partial
  unique index — so the invoice itself just stays `open`. That kept the state machine clean."
- How invalid transitions are rejected: validated against `can_transition_to`, then applied with a
  status-conditional `UPDATE ... WHERE state = $from` — an illegal one returns **409
  invalid_state_transition**.

**Achieve:** you sound like you designed it — you can say why each state exists and what's terminal
without reading.

> ⚠️ Glance at the diagram, then look away and *explain* it. Don't read the table.

---

## SECTION 4 — Failure-mode walkthrough  (~2:00)   [UNSCRIPTED — open the code]

**Pick the PSP timeout (case b).** It's the strongest and you can even demo it.

**Optional 15-sec live demo first:** in Postman run **3. Payments → TIMEOUT (tok_timeout)**.
> "The PSP sleeps 30 seconds, but my client gives up at 5 — and you can see it returns **202** in
> a few seconds, not 30. Status is `pending`."

**Then open the code and walk the lines** (`payment_service.rs`):

1. **The `pay` function** — point at Txn 1 (claim idempotency key + insert the `pending` attempt),
   then the `state.psp.charge(...)` line.
   > "Two transactions. I commit the pending attempt first, *then* call the PSP — I never hold a DB
   > transaction across this HTTP call."
2. **`psp/mod.rs`** — point at `if e.is_timeout()` → `PspResult::Unknown`.
   > "A timeout isn't a failure — it's an **unknown** outcome. I genuinely don't know if money
   > moved."
3. **`finalize`, the `Unknown` branch** — point at "leave attempt pending, set `next_poll_at`,
   return 202".
   > "So I leave the attempt `pending` and return 202 — I refuse to guess."
4. **`migrations/0008_payment_attempts.sql`** — point at `uq_active_attempt_per_invoice ... WHERE
   status IN ('pending','succeeded')`.
   > "While it's pending, this partial unique index blocks any second attempt — so no double
   > charge during the unknown window."
5. **`reconciler.rs`** (one line):
   > "And a background reconciler re-queries the PSP with the **same idempotency key**, which the
   > PSP dedupes on — so it resolves the attempt without ever charging twice."

**Achieve:** you've shown, in the actual code, exactly how an unknown PSP outcome is handled
without hanging the API or double-charging.

**Close (10 sec):**
> "So: integer money, concurrency-safe single-charge, idempotent retries, and failure modes that
> never corrupt state — all explained in DESIGN.md. Thanks for watching."

---

## Final checklist
- [ ] Showed `docker compose up` + healthz
- [ ] Created customer, invoice (proved server-side total), success pay, declined pay
- [ ] Showed webhook deliveries (events + live signed webhook)
- [ ] Explained the state machine in my own words
- [ ] Walked ONE failure mode through the real code
- [ ] Total 5–10 min
- [ ] Paste the Loom link into README.md → "## Demo Video"
