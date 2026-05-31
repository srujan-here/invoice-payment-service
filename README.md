# Invoice & Payment Service

A minimal billing backend in **Rust (Axum + sqlx + Tokio + PostgreSQL)**. Businesses create
invoices for customers, customers pay them through a mock PSP, and businesses receive **signed,
retried webhooks** on state changes. Money is integer cents, USD only — no floats anywhere.

- **[DESIGN.md](DESIGN.md)** — the primary design document (data model, state machine, payment
  correctness & failure modes, webhooks, API keys). Read this first.
- **[AI_USAGE.md](AI_USAGE.md)** — how AI was used on this assignment.
- **[docs/openapi.yaml](docs/openapi.yaml)** — full API spec (request/response shapes, error format).

## Demo Video

The walkthrough is split into two parts (each viewable without login):

- **Part 1 — Architecture & Live Demo:** https://www.loom.com/share/9838182356aa41a38afc051d05e57317
- **Part 2 — State Machine & Failure Mode:** https://www.loom.com/share/b44932603433475692a59cb3f26041bd

Part 1 covers the architecture overview and a live `docker compose up` demo (create customer →
create invoice → successful payment → `tok_card_declined` payment → signed webhook deliveries).
Part 2 walks the invoice state machine and one failure mode (the PSP timeout) through the code.

## Run it

Requires only Docker. One command brings up Postgres, the mock PSP, and the service (migrations run
automatically on startup — no manual steps):

```bash
docker compose up --build
```

- API: `http://localhost:8080`  ·  Mock PSP: `http://localhost:9090`  ·  Postgres: `localhost:5432`
- Health check: `curl http://localhost:8080/healthz` → `ok`

A demo business and API key are **seeded by the migrations**, so the API is usable immediately:

```
API key: dpk_test_seedkey0000000000000000000000000000
```

## Curl examples

```bash
KEY=dpk_test_seedkey0000000000000000000000000000
API=http://localhost:8080
AUTH=(-H "Authorization: Bearer $KEY" -H "Content-Type: application/json")

# 1) Create a customer
curl -s "${AUTH[@]}" -d '{"name":"Alice","email":"alice@example.com"}' $API/v1/customers
#   -> {"id":"<customer_id>", ...}

# 2) Create an invoice. The server computes the total from line items;
#    any client-supplied "total" is ignored.
curl -s "${AUTH[@]}" -d '{
  "customer_id":"<customer_id>",
  "line_items":[{"description":"Pro plan","quantity":3,"unit_amount_cents":1500}]
}' $API/v1/invoices
#   -> {"id":"<invoice_id>","state":"draft","total_cents":4500, ...}

# 3) Finalize (draft -> open) so it can be paid
curl -s "${AUTH[@]}" -X POST $API/v1/invoices/<invoice_id>/finalize

# 4a) Pay — SUCCESS. Idempotency-Key is required.
curl -s "${AUTH[@]}" -H "Idempotency-Key: pay-1" \
  -d '{"card_token":"tok_success"}' -X POST $API/v1/invoices/<invoice_id>/pay
#   -> 200 {"status":"succeeded","invoice_state":"paid","psp_ref":"..."}
#   Replaying the SAME key returns the same result with no second charge.

# 4b) Pay — FAILURE (declined). Invoice stays "open" and is retryable.
curl -s "${AUTH[@]}" -H "Idempotency-Key: pay-2" \
  -d '{"card_token":"tok_card_declined"}' -X POST $API/v1/invoices/<another_invoice_id>/pay
#   -> 402 {"status":"failed","failure_code":"card_declined","invoice_state":"open"}

# 5) Register a webhook endpoint (returns the signing secret once)
curl -s "${AUTH[@]}" -d '{"url":"https://example.com/webhooks"}' $API/v1/webhook-endpoints

# 6) Reconciliation: see the events that occurred and their delivery status
curl -s "${AUTH[@]}" "$API/v1/events"
curl -s "${AUTH[@]}" "$API/v1/webhook-deliveries"
```

### Mock PSP card tokens

| Token | Behaviour |
|---|---|
| `tok_success` | succeeds after ~100 ms |
| `tok_insufficient_funds` | fails (`insufficient_funds`) |
| `tok_card_declined` | fails (`card_declined`) |
| `tok_timeout` | sleeps 30 s — our client times out first; the payment goes `pending` (202) and the reconciler resolves it |
| `tok_network_error` | returns 500 / drops the connection — treated as unknown, goes `pending` |

## Tests

```bash
cargo test          # unit tests (money, state machine, signing, key hashing)
cargo test --test payment_flows -- --test-threads=3   # the three required integration tests
```

The integration tests use **testcontainers** to spin an ephemeral PostgreSQL (needs Docker), boot
the app in-process against a call-counting mock PSP, and cover:

1. **Concurrency** — N simultaneous `/pay` for one invoice → exactly one succeeds, PSP charged once,
   invoice ends `paid`.
2. **Idempotency** — same key+body twice → identical response, no second PSP call.
3. **PSP failure** — `tok_timeout` → endpoint returns `202` promptly, invoice not stuck/corrupted.

## Architecture

```
crates/
  common/           # Money (integer cents), AppError + JSON error envelope, UUIDv7 ids, config, pagination
  invoice-service/  # Axum app — routes -> services -> repositories -> db
      auth/         #   API-key middleware (SHA-256, prefix lookup, constant-time compare)
      services/     #   business logic, invoice state machine, payment flow, transaction orchestration
      psp/          #   PSP HTTP client (bounded timeout, forwards idempotency key)
      webhooks/     #   HMAC signing + background dispatcher (outbox, retries)
      reconciler.rs #   background task resolving pending payment attempts
  mock-psp/         # standalone mock payment processor
migrations/         # forward-only sqlx migrations (embedded; run on startup)
```

Two background Tokio tasks run alongside the HTTP server: the **webhook dispatcher** and the
**payment reconciler**, both with graceful shutdown on SIGTERM/Ctrl-C.

## Configuration

All via environment (see `.env.example`); `docker-compose.yml` sets them for you. Notably
`PSP_TIMEOUT_MS` (default 5000) is the PSP client deadline — deliberately shorter than the mock's
30 s `tok_timeout`.

## Error format

Every error is `{"error":{"code","type","message","details?}}` with a stable, snake_case `code`.
See [docs/openapi.yaml](docs/openapi.yaml).
