# Quiet element sync

Standalone editable projects sync 3 s after the last tracked edit and after opening
with pending element work. Local saves still use the existing deferred write queue.
The Settings switch `Sync automatically` defaults to on. The separate
`Sync to the database when saving` switch retains its existing behaviour.
Hosted projects continue to use the shell flow. Read-only projects, projects without
edit access and projects waiting for a choice do not sync automatically.

| Status | Meaning |
| --- | --- |
| Saved | Local work is waiting, or a current server revision has not been established. |
| Syncing… | A check or sync is running. |
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
`SyncAttention.svelte` provides the shared prompt framing for the later atlas
replacement work; this change does not implement or confirm `replace_confirm`.

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
