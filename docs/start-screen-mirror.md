# Start-screen mirror

The start-screen metadata and thumbnail cache follows sections 1–3 of
[UPbDB ADR-0012](../../upbdb/docs/decisions/ADR-0012-mirror-mode.md).
The mirror-mode decision of 2026-11-17 replaces the earlier instruction to
clear this cache on sign-out.

Desktop stores these tiles in
`<app data>/mirror/<dataset_id>/<account_id>/start-screen.json`. Until
`GET /v1/server` reports `dataset_id`, the dataset key is the SHA-256 hash of
the normalized server URL, as in UPb Lab: URL parser normalization, no query,
fragment or trailing slash. When the dataset ID appears, the same account's
URL-keyed copy is carried into that dataset's folder. The original copy is
retained. A standby reporting the same dataset ID uses the same mirror.
Browser and hosted builds use these paths as IndexedDB keys in
`grainatlas-mirror`.

The budget remains 500 tiles and 20 MB per account mirror for this start-screen
cache. Optional thumbnails are evicted before metadata. This budget does not
apply to projects, stored packages, drafts or pending edits.

Sign-out hides the server tiles and invalidates requests still in flight. It
keeps the mirror and all unsynced local work. The signed-out screen shows local
projects and “Sign in to see your mounts”. Signing in again resolves the account,
shows its mirrored tiles immediately, then revalidates with ETags. A known
credential can show its bound account's mirror before the identity request
finishes, including offline. A new credential must resolve `/v1/accounts/me`
first. Credential hashes and account/dataset bindings live in keyed
`mirror-bindings/<server hash>/<credential hash>.json` entries. Tokens are never
stored there. No account folders are enumerated and no last-account fallback
exists. Desktop mirror folders use mode 0700 and files use mode 0600.

When unreachable, the tiles say “Offline, as of <time>” and allow local actions
only. Lists remain paged; “Show more” follows `X-Next-Cursor` and each page keeps
its ETag. Full metadata mirroring, the change feed and heavy-file mirroring are
separate work under ADR-0012.

Lab chips come from the paged `/v1/labs` response. Ordinary accounts see a chip
for each returned lab. Superusers see labs whose `my_role` is `owner` or `member`,
default to the first such lab, and have an “All labs” chip. Older servers without
`my_role` fall back to “Mine” and “All labs”. Team membership `lab_id` values are
not used to infer lab membership.

Identity and dataset reads share an in-memory promise per normalized server and
signed-in credential. Concurrent consumers, screen changes, project opens and
syncs reuse the result for the app session. Failed reads are retained too;
sign-out or a credential change permits a fresh lookup. Neither read runs without
a server and token. These background reads do not gate local editing. Lab and
mount lists continue to revalidate separately with ETags.

Unit tests cover identity and server isolation, retained mirrors and drafts,
repeat sign-in, offline identity, URL-to-dataset migration, standby reads, cache
budgets and labs with and without `my_role`. Playwright covers the visible
sign-out and sign-in transitions, offline local actions and lab chips at both
supported window sizes and themes.

On the 4000-lab mocked list, chip discovery still transfers just the first 100
labs. Dataset discovery adds one request: 2 before, 3 after (0.33 ms and 0.38 ms
in the unit-test run; these measure mocked processing, not network latency).
There are no requests per lab. The 4000-mount benchmark transfers 50 rows on
the first request instead of 4000 (about 0.43 ms instead of 5.16 ms in that run).

The round 2 five-consumer mock compares uncached discovery with reuse in one
sign-in: identity requests fall from 10 to 2, and total requests from 15 to 7
(the five lab-page revalidations remain). Mocked processing took 1.88 ms and
1.10 ms respectively; these are not network timings. The 4000-lab and
4000-mount tests still enforce bounded pages. Bundle-based Playwright mocks
share invented identity, dataset and empty lab-page responses; their
unexpected-request guards remain intact. An additional request-count assertion
allows at most one read of each identity endpoint per document, server and
credential. A full page reload starts a new app session and may read both again.

Round 3 keeps the revision badge lookup independent of mount loading. It runs
only on the visible Open screen with a server, token, account and at least one
recent project whose local draft links to a mount on that server. Drafts which
are not in Recent projects and unlinked drafts do not trigger it. Filtering or
paging the mount list does not trigger another revision lookup.

Each sorted, deduplicated batch contains at most 500 project UUIDs. Larger recent
lists use multiple batches without dropping UUIDs. Each batch retains its own
ETag and check time, scoped to normalized server URL, account, view-as identity
and exact request body. Answers younger than 60 s are reused across screen
changes and, through the account mirror, document reloads. Expired answers send
`If-None-Match`; a 304 retains the rows and renews the check time. Concurrent
consumers share a pending request. Leaving Open stops subsequent batches; a
session change discards in-flight results. Mount loading, Refresh and navigation
do not await the lookup.

The shared synthetic-server helper now handles only the exact read-only
`POST /v1/atlas/projects/revisions` endpoint, with empty or invented rows, an ETag
and a 304 for matching `If-None-Match`. The existing unexpected-request arrays
and assertions are unchanged. Bundle specs print revision request counts and
completed response time alongside their existing identity measurements.

The supplied round 2 log reports these requests reaching the guard (line numbers
refer to that run). Every row shares the missing mock route as its failure cause.
The former lookup also continued after the mount-list response without checking
that Open was still visible. The two reopen cases replace the document and
formerly lost the component's ETag and any prospect of freshness reuse.

| Spec and case | Revision POSTs before | Additional cause | Revision POSTs after |
| --- | ---: | --- | --- |
| print-sheets:99, 1440 x 900 | 1 | Mount-list continuation | Not measured |
| print-sheets:99, 1280 x 800 | 1 | Mount-list continuation | Not measured |
| remote-edit:163, create and select a mount | 1 | Mount-list continuation | Not measured |
| remote-edit:210, published-image editing | 1 | Mount-list continuation | Not measured |
| remote-edit:278, publish tone and reopen | 2 | Document replacement repeated the lookup | Not measured |
| spot-size:17, ordinary identity response | 1 | Mount-list continuation | Not measured |
| strip-editor:207, downloaded project reopens | 2 | Document replacement repeated the lookup | Not measured |
| strip-editor:260, unreadable edits | 1 | Mount-list continuation | Not measured |
| strip-editor:386, refused thumbnail drop | 1 | Mount-list continuation | Not measured |
| strip-editor:456, refused drop at 1440 x 900 | 1 | Mount-list continuation | Not measured |
| strip-editor:471, problems at 1440 x 900 | 1 | Mount-list continuation | Not measured |
| strip-editor:456, refused drop at 1280 x 800 | 1 | Mount-list continuation | Not measured |
| strip-editor:471, problems at 1280 x 800 | 1 | Mount-list continuation | Not measured |

These are 15 guard-rejected requests across 13 failed cases; the supplied log
does not provide revision response times. Browser after counts remain unverified:
this session cannot bind the preview server to port 4173 (`listen EPERM`). No
browser pass count is claimed. Actual POST counts may be nonzero when Open is
visible with eligible recent projects; the target is correct eligibility and
freshness, with no unexpected requests, rather than suppressing legitimate reads.

The five-visit unit benchmark makes 5 uncached revision requests before and 2
after (one initial answer and one expired conditional lookup), taking 0.49 ms
and 0.27 ms respectively in the recorded run. These are mocked processing times,
not network latency. The 4000-project test previously accepted one 500-UUID
request and silently omitted 3500 projects. Its strengthened assertion requires
8 requests covering all 4000 UUIDs, with 0 extra requests for a fresh revisit
(4.34 ms for both visits). Additional tests cover separate ETags for multiple
batches, 304 row retention, scoping, coalescing, failed responses, ineligible
projects and navigation or sign-out during a stalled lookup.

Validation for round 3: `make full` passed (144 Rust tests, 35 Node tests,
42 Vitest tests, desktop compilation and Svelte checking with no errors or
warnings). The static build passed. Playwright discovers 90 cases, including
five new regression cases; execution is blocked by the preview port restriction.
The desktop app was not launched, as required by the repository instructions.
