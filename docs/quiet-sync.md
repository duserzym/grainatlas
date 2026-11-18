# Quiet sync

Standalone editable projects sync 3 s after the last tracked edit and after opening
with pending element work. Local saves still use the existing deferred write queue.
The Settings switch `Sync automatically` defaults to on. The separate
`Sync to the database when saving` switch retains its existing behaviour.
Hosted projects continue to use the shell flow. Read-only projects, projects without
edit access and projects waiting for a choice do not sync automatically.

| Status | Meaning |
| --- | --- |
| Saved | Local work is waiting, or a current server revision has not been established. |
| Syncing… | A check or element sync is running. |
| Syncing images… | An image bundle is being built, uploaded or acknowledged. |
| Up to date | No work is pending and a checked server revision matches the local base. |
| Offline, will sync later | A request could not complete because of a network or server failure. |
| Needs your attention | A confirmation, conflict, held item or non-transient error needs a person. |

The chip and the Open list use `plainSyncStatus()` in `sync-status.ts`. Open rows
belonging to an open tab use that session's live status. Version, revision, counts
and raw failures stay in Details and the chip title. Stored revision checks expire
after 10 min. A revision lookup never acknowledges local edits.

Network failures and 5xx replies retry after 5 s, 10 s, 20 s and so on, capped at
5 min. Each delay has ±20 percent jitter, with a hard maximum of 5 min. Success
resets the backoff. Online and window-focus events wake pending sync immediately.
There is only one sync in flight per session. Background failures do not put a
transient error in the main error banner.

Every sync, link, mount creation and zip publish sends `Idempotency-Key`.
A 409 `idempotency_in_progress` waits for `Retry-After` seconds (2 s by default).
After 30 waits, sync returns to transient-failure backoff with the same pending key
and body for the next attempt.
A 422 `idempotency_key_reused` renews the key once, logs the renewal and retries.
Ordinary 409 confirmations pass through unchanged. Multipart publishing encodes its
boundary and body once for the action's retries.

Before a real sync request leaves the app, the versionless draft JSON durably
records its ID, exact body, submitted elements and acknowledgement context in
`pending_request`. A retry, including after restart, sends that saved request
before considering newer edits. Key renewal is persisted before retry. The legacy
`request_id` field stays stable across an ADR key renewal too, so the encoded body
also stays stable. The pending request clears only after the accepted elements
have been checked, or after a definitive refusal. Later edits remain pending.
Storage failure stops the send. App updates do not change draft storage keys.

Independent remote changes, including different fields on the same row, combine
through the existing three-way merge. Actual conflicts remain explicit choices.
A registered spot cannot use the separate-copy resolution. Bulk removal always
requires a person's choice, with the spots consequence shown in the prompt.
`Keep them (recommended)` restores the targeted grains and spots locally from the
database copy, clears the confirmation and resumes automatic sync of other edits.
Only `Remove and sync` sends `confirm_bulk`.
`SyncAttention.svelte` also frames image replacement decisions. Only an explicit
Replace choice sets `replace_current: true`; keeping the images suppresses that
snapshot until the local pixels change again.

## Server proposal: batch revision checks for Open

The current server reference has a per-project lookup and a candidate search,
but no batch revision lookup. Automatically looking up every pending draft would
violate AGENTS.md's rule against requests per row. Open therefore reuses stored
checks and open-tab sessions, and retains explicit Check for unchecked copies.
Automatic checks once per Open visit remain unfinished pending this server addition.

Propose a read-only revision summary endpoint accepting a bounded collection of
project UUIDs and returning UUID, mount ID, revision and edit permission for visible
projects. It should accept `limit`, return `X-Next-Cursor`, support `ETag` /
`If-None-Match`, and avoid revealing inaccessible UUIDs. The client can then check
only listed drafts with local work, batch them, request pages sequentially once
per visit, persist `checked_at` and `server_revision`, and reuse them for 10 min.
No files in the server repository were changed.

## Verification and request measurements

`make full` passes, including Rust tests, desktop compilation, Svelte checks and
94 UI unit tests. The browser scenarios cover automatic placement sync,
two 503 replies, keeping a bulk removal and reload with a held reply. The E1b
Playwright run could not start because the environment rejected the preview
server's port bind with `EPERM`. Browser verification and the desktop app smoke
test remain with the supervisor.

A local mocked recovery experiment used the original HEAD session and the new
session, 30 runs each, with the same synthetic spot and a 503 followed by success.
Only the successful recovery phase was timed; setup and the initial failure were
excluded. Original recovery made 4 requests and averaged 0.36 ms. New recovery
made 3 requests and averaged 0.33 ms. This measures local orchestration, without
network latency or disk I/O, and does not predict relay throughput. The normal
success path retains one sync POST and its existing canonical verification.

The 4,000-draft unit check makes 0 requests and took 1.6 ms in the final full run.
ETag tests verify conditional detail requests, 304 reuse, credential separation
and clearing on sign-out. These checks do not measure a live server.


## Background images

Changed pixels publish after 10 s without edits, after element sync settles. The
window must be visible, with no attention prompt, modal task, live render or Area
image download running. Editing pauses the next file or part until 10 s idle.
The existing automatic-sync preference and hosted/read-only gates still apply.
Explicit Publish uses the same engine and retains the advisory checks.
An older copy pauses image publishing too. Background images require a successful
database check and wait while the database is known unreachable. The gates are
checked again after bundle preparation and between requests, including retries
and acknowledgement of a completed upload.

The core builds its usual STORE zip in the wasm worker or native blocking task,
without the task modal. The client extracts its files and checks every manifest
size and SHA-256 with asynchronous Web Crypto. The draft's `publish_job` keeps a
stable export time, credential fingerprint, server/mount, manifest hash, upload ID,
creation/commit/zip keys, count and expiry. Tokens and image bytes are not in the
draft. Critical identity changes commit to draft storage before sending a write;
ordinary progress uses the deferred write queue. A storage failure stops sending.

A session-cached empty asset check selects the new routes. Missing hashes upload
with two workers and parts of 8 MiB for larger files. A restart rebuilds and checks
the manifest, revalidates upload status with its ETag, and sends only the remaining
hashes. Changed bundles and expired uploads start again; server blobs remain
reusable. Each exact part has a reproducible UUID derived from the creation key,
its hash and offset. Incomplete commits repair the listed hashes and use a fresh
commit key because ADR-0010 caches 4xx replies. A durable successful reply allows
local acknowledgement to resume without publishing another version.

Unavailable routes (404, 405 or 501) select zip publishing. A network failure at
capability check pauses publishing until a successful database check; it does not
cache an unsupported capability or start a zip upload. The zip's multipart boundary
and export time survive a restart, keeping the encoded bytes and Idempotency-Key
stable. Both paths share overlay baking,
recordPublished, published-project pixel acknowledgement, revision refresh and
UPb Lab notice. Later local edits remain pending. Image failures remain in the
status details and retry; image replacement alone requests an attention decision.
Capability, session, blob, status, commit and zip requests use the existing HTTP
helpers, with upload-scaled write deadlines. The header-only helper returns the
original response; no response-body stream wrapper is added. A timed-out JSON
reply remains a retryable image failure rather than a cancelled user action.

Published file downloads have a 10 s response-header deadline and a 30 s deadline
without bytes, with no overall transfer deadline. A failed file retries up to
three times with 1 s, 2 s and 4 s delays. Sign-out stops readers before clearing
the disposable image cache and upload-status cache.

## Server proposals for ADR-0010 section 2

The routes themselves are sufficient, subject to these details being made
explicit and implemented. No server files were edited.

- Bound every `missing` array to 5,000 distinct hashes. A subsequent status or
  incomplete-commit reply must return the next remaining window after accepted
  blobs disappear from the missing set, until all are present. Keep received/total
  counts accurate for manifest paths, including repeated content hashes, and
  include received/total in creation replies as well as status replies. The
  client consumes successive incomplete windows; it does not require an unbounded
  missing response or make a check request per tile. If cursor pagination is
  preferred instead, its wire fields must be specified before this client can
  follow them.
- A chunk acknowledgement must return `308` with `Range: bytes=0-N` and no
  Location header. After a crash, a repeated prefix must acknowledge the retained
  contiguous offset rather than reject overlapping bytes. A full existing blob
  returns 200/201. Include Content-Range in request identity validation (or otherwise
  ensure a reused part key cannot silently describe another range).
- Browser CORS must allow Authorization, Idempotency-Key, Content-Type,
  Content-Range and If-None-Match, and expose Range, ETag, Retry-After and
  Idempotent-Replayed. Proxies must preserve 308 and Range without redirecting.
- Commit must recheck replacement permission if another project's images became
  current after session creation. Return 409 `replace_confirm` in that race; the
  client asks again and creates a new session with fresh keys after Replace.

The added browser scenarios cover changed-tile deduplication, reload during a held
PUT and zip fallback. They are prepared for the supervisor; browser execution,
desktop smoke testing and a live implementation of the planned routes remain
unverified. The 4,000-file unit mock compares one zip write (400,000 invented
bundle bytes) with five requests including capability, creation, two one-byte
blobs and commit. It asserts the request budget and records local orchestration
time; these timings do not predict network throughput or count JSON overhead.
