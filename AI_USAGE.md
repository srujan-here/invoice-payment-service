# AI_USAGE

I used an AI coding assistant (Claude, via an agentic CLI) throughout this assignment. Below is a
specific, honest account.

## What I used it for

- **Design pressure-testing.** Before writing code I had the assistant critique my leaning design
  for the three hardest areas in parallel — the payment concurrency model, the webhook delivery
  system, and the data model. The most useful output was *push-back*, not generation (see below).
- **Scaffolding and boilerplate.** Cargo workspace wiring, the `IntoResponse` error plumbing,
  repository CRUD queries, the Axum route handlers, and the `docker-compose.yml` / `Dockerfile`
  were largely AI-drafted, then reviewed line-by-line and adjusted.
- **Migrations and SQL.** Draft table definitions and indexes, which I then revised (e.g. tightening
  the partial unique index predicate, adding the `GENERATED STORED` column).
- **Explaining trade-offs.** I had it lay out `SELECT FOR UPDATE` vs advisory lock vs `SERIALIZABLE`
  vs optimistic version vs status-conditional update so I could choose deliberately and write §3 of
  DESIGN.md in my own framing.
- **Test harness.** The testcontainers + in-process-app + call-counting-mock-PSP harness was
  AI-assisted; I specified what each test had to assert.

## Three decisions I made myself (against or independent of the AI)

1. **No transient `charging` invoice state.** An early AI sketch proposed adding a `charging` state
   to the invoice machine to represent an in-flight payment. I rejected it: it complicates the state
   machine the assignment specifies, and a `pending` row in `payment_attempts` (pinned by the
   partial unique index) already represents "in flight" as a single source of truth. The invoice
   stays `open` until a definite outcome. This kept the state machine clean and made the failure-mode
   reasoning simpler.

2. **SHA-256 for API keys, not argon2/bcrypt.** A common AI default is "hash secrets with a slow
   KDF." I chose plain SHA-256 deliberately: API keys are 200+ bits of CSPRNG entropy, so a slow KDF
   buys no real security, needs no salt, and would burn CPU on *every* request. I documented the
   reasoning (and the HMAC-pepper hardening I'd add) in DESIGN.md §5 rather than cargo-culting bcrypt.

3. **Runtime-checked sqlx queries over the compile-time `query!` macros.** The macros give
   compile-time SQL verification but require a live database (or a committed offline cache) at *build*
   time, which fights a clean `docker compose up` and hermetic CI. I traded compile-time checking for
   build hermeticity and used `query_as::<_, T>().bind()` throughout — a conscious call, noted here.

## One thing the AI got wrong (and how I caught it)

The integration tests failed on migration 7 with `syntax error at or near "("`. The AI's first
instinct was that the `GENERATED ALWAYS AS (...) STORED` column was malformed. I verified the
migrations were actually valid by running them directly against `postgres:16` (they applied cleanly)
and then re-running the sqlx migrator against a pinned PG16 container (also clean). The real cause:
**`testcontainers` defaults to an old PostgreSQL image (pre-12) that doesn't support generated
columns**. The fix was to pin the test container to `16-alpine` to match `docker-compose.yml`, not
to change the SQL. The lesson — and why I verify rather than trust — is that the AI proposed changing
*correct* code to chase an environment problem; isolating the variable (run the same SQL against a
known-good server) surfaced the actual cause.
