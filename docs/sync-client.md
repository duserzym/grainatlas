# Database sync client (B2–B4)

The client follows laicpmsdb B1, including completeness updates `90555db` and `5f38c3b`, based on `3f63ee9` and `docs/atlas-
sync.md`: `ui/src/lib/sync-api.ts` owns its URLs, authentication, wire types and
mappings. The client regression tests use the small fixture and an in-process fake. A live
scratch database exposed the legacy publish state covered below.

## Elements and revisions

A revision belongs to a mount. Several project uuids may share it. The client
never adopts a different project uuid automatically. A package-name match is
available only through the Not in the database chip. Its popover offers Link and
Not this one; it never opens itself or blocks the page. The draft remembers the
lookup and dismissed offers. Candidates use `GET
/v1/atlas/projects?package=&name=`, sending both known names. An explicit link
posts `{mount_id, make_current: false}` to
`/v1/atlas/projects/{local_uuid}/link`; it keeps the local UUID and the current
published atlas. Candidate UUIDs are never adopted. A link 409 displays “This
project is linked to another mount”, clears the offers and disables
registration. The draft remembers this refusal. `linked_uuid` remains readable
only for explicit links stored by the provisional client.

Lookup reads AtlasProjectOut, including mount revision, atlas version, counts,
hashes, nullable changed_at/by, can_edit, current_project_uuid and the linked
project_uuids list. Several UUIDs can remain live; the current published UUID
does not supersede another project. With can_edit false, the chip says “Read-
only access”; Sync, sync-on-save and hosted sync writes are disabled with a
permission reason. Local edits can still be kept.

`GET /v1/atlas/projects/{uuid}/elements` returns MountElementsOut, including
`{mount_id, revision, project_uuids, current_project_uuid, spots, grains,
labels, hashes}` for that uuid and project-less elements on its mount. It
supplies current elements only. A saved older base plus `GET
/v1/mounts/{id}/changes?since=N` supplies three-way review. Legacy drafts
without a base compare against current `/elements`, but lookup never establishes
a deletion baseline for content the client did not display. History entries belonging to other project uuids never enter the
project's merge.

These are the narrowed stored fields. Sample is a snapshot hint and does not enter equality:

| Kind | Fields |
| --- | --- |
| Spot | id, name, area_id, grain_id, x_um, y_um, x2_um, y2_um, diameter_um, kind, comment |
| Grain | id, area_id, number, label, polygon_um, centroid_um, area_um2, sample, artifact, split_from, merged_from |
| Label | id, area_id, x_um, y_um, text |

A missing grain label defaults to `<area_id>/<number>`. Grain sample comes from
its area when the local grain has no explicit sample. Sample hints are sent only
in snapshots; unknown or ambiguous names can return null and never make the
local diff dirty. Geometry numbers compare with a relative tolerance of 1e-6
(and an absolute floor of 1e-6 near zero). Empty/null comments and artifact
values, optional endpoints and default spot kind are normalized. Artifacts use the write contract's empty
string default. Grains are flat on the wire, grouped by area in the UI. The
bundle or existing local copy supplies UI-only measurements; narrowed server
geometry supplies bounds and fallback measurements for new grains.

`server_id`, `project_uuid`, `acquisition_ids`, `retired`, `children`, and UI-
only metadata never enter ordinary upserts or stored-field equality. Read
lineage `split_from: [uuid]` becomes a scalar UUID; an empty list becomes
absent/null. Merged parent lists are sorted for equality and remain lists on
writes. Retired grains and annotations are excluded from active elements.

Legacy grains with id null and server_id remain in the base. Diffing and review
match them to existing local UUIDs by label (or area/number); server IDs never
become local identities. Their missing geometry remains a local change. Opening in View overlays
current identified server rows on the bundle, hides retired grains and labels,
and keeps bundle rows missing on the server for the first snapshot. Legacy
rows preserve matching bundle geometry and spot links until adoption. A pending
draft instead supplies the displayed content, with “Your draft here, based on
r<n>”. Its displayed change counts compare against current server elements,
while its older base stays intact for three-way review. Editable content retains local edits; post-sync refetches acknowledge canonical
fields and kept omissions while preserving edits made in flight. Explicit
Update from database or a chosen merge applies later server changes. Server IDs returned
by adoption remain metadata; they never replace client UUIDs. Acquisition links
still prevent the unsafe Both choice for a conflicting spot.

Legacy id-null grains matching local labels, or missing server coverage on
opening, require a first snapshot even if an old draft says it synced. Mount-wide
legacy counts and unrelated id-null rows do not force repeated snapshots. View and Edit show “Not yet synced · 25 grains, 3 spots” for the fixture.
All 25 detections are kept and sent: 20 grains and five flagged artifacts (which
retain number 0 and get stable unique artifact/UUID labels within B1’s
50-character limit). The first sync sends `mode: "snapshot"` and every local grain,
spot (including grain_id) and label. Subsequent syncs send `mode: "delta"` with
only changed elements. Removals use the last-seen rule below. A snapshot must
cover every active identified row of this project, through an upsert or explicit
delete; omitted foreign and id-less context remains in `not_in_snapshot`. Later server revisions still require an update or merge. Every grain delete
retires the grain, preserving server identity. Split/merge additions carry
`split_from` / `merged_from`; every spot on a parent must be reassigned in the
same request. After an accepted write the client refetches canonical elements,
especially after legacy adoption, and sets the base only for the acknowledged
revision.

The 200 reply contains `mount_id`, `project_uuid`, `mode`, `previous_revision`,
`revision`, `applied`, per-kind `ids`, `acquisitions_linked`, `warnings`,
`rebased_over`, `complete`, and `not_in_snapshot`. `ids` maps stable client uuids to
database server ids; it does not rename the local elements or their references.
A stale base caused solely by other projects can succeed with numeric
`rebased_over`. Warnings and rebasing produce a notice. Own identified omissions
block acknowledgement; foreign and id-less omissions are context only, as below. `acquisitions_linked` is also numeric; `applied` contains per-kind
counts.

A 409 reply contains `detail`, `base_revision`, `current_revision` and `changes:
[{revision, kind, id, op, data, project_uuid, at, by}]`. The transport converts
that ordered event list to the three-way input, including deletion and
retirement. Review lists every change and requires explicit choices for
collisions. Both can duplicate annotations and differently named spots without
acquisition links. Geometry and deletion/edit conflicts require Mine or Theirs.
Another concurrent write produces another review. A 422 with detail messages
keeps the draft and its old base, displays every message in the chip error, and
stays reachable. Only `snapshot_incomplete` receives one automatic retry, as below. Hosted command results use code
`failed` and the first message. Schema detail objects use their msg field.

Deleting a spot with known server acquisition_ids is refused at once with “This
spot has measurements in the database; it cannot be deleted there”. Checks
refresh measurement links even when content hashes are unchanged. Protected
spots are excluded from delete diffs; an older draft or undo that already
removed one restores it locally before sync. The server still enforces
measurement protection when links change between check and write.

## Last-seen removals and completeness

A removal means last-seen minus local, never current server inventory minus local.
Last-seen is the acknowledged `draft.base`, or the displayed server overlay
`viewBase` recorded as `draft.base_seen` when Edit begins. A fresh editable file
also records its displayed elements on open; only matching owned server identities
can be removed. A clean View entering Edit can establish a base before the first
client sync. Pending legacy drafts retain their older base and last-seen set.
A legacy draft with neither base nor base_seen sends no inferred removals.
Grain removals retire, rather than erase, the server grains. Measurement guards
and the dry-run bulk confirmation apply to every delete source.

An own active identified server row absent both locally and from last-seen is a
database addition. The chip says “Database has more” (both-changed), and the drawer
says “The database has N grains/spots/labels this copy does not”. Keep them pulls
the batch locally. Delete them… first asks for confirmation naming its count,
then sends explicit deletes through the normal bulk guard. Selecting one batch
never establishes deletion intent for another kind.

Before any sync, unresolved database additions are automatically kept: they are
merged into the local copy and sent unchanged in snapshot upserts. This is the
chosen conservative policy: keeping existing database content loses no data and
avoids an incomplete snapshot. The drawer records them as “added from the
database” in `database_added`, with Keep them and the explicit Delete them…
actions still available. These displayed additions join last-seen, so a later
user removal is an ordinary explicit deletion. A base-null legacy copy follows
the same policy. Only additions may be rebased automatically; changes to existing
seen elements still require review. Edits made during requests remain pending.

`complete: false` with `not_in_snapshot` containing own identified elements is
not synced: retain the draft/base, list the left-out elements and their Keep them /
Delete them… actions, and return hosted `ok: false, code: 'failed'` with the reason.
Other projects' elements and atlas-id-less legacy rows are listed as “kept by the
database (not part of this project's copy)” without delete actions. Those context
rows do not block In sync even when the server returns complete false. A successful
write is still checked against the submitted intent and a canonical refetch.

A 422 with `code: 'snapshot_incomplete'` has `missing: {spots, grains, labels}`.
Refetch and rebuild once: missing last-seen identities absent locally go into
delete lists; unseen identities are kept/pulled into the snapshot. The retry
previews again and retains the bulk guard. This applies whether the refusal came
from preview or the write. A second refusal displays the message and stops with
the draft intact; hosted returns `ok: false, code: 'failed'`. Other 422s carry
`code: null, missing: null` and do not retry.

## Hashes

B1 hashes each non-retired element with a non-null atlas ID with SHA-256 over
Python `json.dumps(el, sort_keys=True, separators=(",", ":"))`. A kind hash is
SHA-256 of `<id> <element sha>\n` lines sorted by id. Empty kinds hash the empty
string. These canonical hashes include project_uuid, retirement and lineage, and
exclude server_id and acquisition_ids. Geometry uses Python float spelling and
strings use ensure_ascii escaping.

The browser never compares its own hash to a server hash. Python and JavaScript
float spellings differ. The client saves opaque project-scoped `/elements`
`base_hashes` at an acknowledged base and compares them only to later
`/elements` hashes. Lookup hashes cover the whole mount, so they must never be
compared to those filtered hashes. Lookup metadata remains the server's
AtlasProjectOut. Local changes come from stored-field element diffs.
Consequently another project's mount revision does not make unchanged elements
look newer. Older draft records without hashes conservatively fall back to the
revision until acknowledged. A later server revision observed after a POST is
not silently acknowledged.

`sync-hash.ts` provides local canonical equality and a test hash helper. The
fake hashes canonical read fields, includes lineage and project UUIDs, excludes
retired/id-null rows, and uses ASCII escaping and integral geometry floats. Its
JavaScript number formatter is not a Python float serialization reference.

## Storage and readiness

`grainatlas.draft.<uuid>` contains uuid, name, server, mount, base_revision,
base elements, base_seen, database_added, incomplete, base_hashes, synced, updated_at, changes, pending, pending_at, source_keys,
linked_uuid, link_offered, link_conflict, link_candidates, pixels_base and pixels_changed.
`grainatlas.drafts` indexes persisted uuids. Existing records with missing
optional fields remain readable.

Draft listing is entirely read-only. Legacy content keys are discovered in
memory; listing does not create draft records or indexes. Opening adopts only
when loading is complete and any Unsaved changes found dialog has been
answered. Background checks and persistence keep that same guard, including
responses that return after a stored-copy choice has appeared. An explicit
start-screen Check can adopt the listed record. Adoption and Check preserve
legacy project bytes and recent-project entries.

Tracking and Save keep unsynced element content in the draft's `pending:
{spots, grains, labels}` and set `pending_at`. These are narrowed upsert shapes,
including new-grain lineage, with no UI-only measurements or acquisition
readbacks. A pending snapshot includes all current elements, so empty arrays
can represent local deletions. When base and current elements match, pending
and pending_at become null. Edits made during a sync remain pending against the
acknowledged base. Offline tracking also retains the current snapshot.

Stored copies still mean unsaved changes absent from the project file. Only
`app.persist()` writes them through `persistProject`; saving clears the scoped
copy and sync never recreates it. Sync code never writes
`grainatlas.copy.<uuid>|<path>`, `grainatlas.project.<package name>` or
`grainatlas.recent`. Source keys are retained for the separate deletion choice.
Start-screen Check and Sync use pending elements, falling back to the newest
stored project only for legacy records with no pending field. A null pending
field never revives an older stored copy. A published bundle supplies display
metadata when available; otherwise pending elements can sync in an empty
session. Reopen raw project files to publish images. Divergent locations still
require Part D reconciliation. Discard removes the draft record, including
pending elements; deleting stored copies requires the separate second
confirmation. Storage exceptions are caught. Files on disk remain under the
normal save workflow.

## Pixels, saving and closing

Element sync and image publication are separate calls. Raw packages with
stitched areas can offer image publishing. Pixel signatures cover mosaic,
scale, stitch, blend, rendering and display tone, excluding segmentation, spots,
labels and print settings. The chip appends New images not published until a
successful publication is confirmed. Failure to publish does not undo element
sync.

Save writes the local file first, then syncs if syncOnSave is enabled and the
registered server is reachable. Unsynced content participates in close prompts
independently of the file's dirty flag. Checks run every 30 s and on online.
A failed network sync can retry after a successful check; concurrent changes
remain behind Review differences. Read-only server viewing preserves an
existing editable draft's base.

## Fake service and verification

`ui/scripts/fake-laicpmsdb.mjs` seeds isolated namespaces from
`target/atlas-fixture`. It models mount revisions, current elements, change
history, snapshot/delta sync, retirement, lineage spot reassignment, per-kind
server ids, AtlasProjectOut/MountElementsOut/ChangeOut, numeric SyncOut fields,
200/409/422 shapes and rebasing over other projects. Unchanged syncs keep their
revision. Snapshot omissions cover the whole mount. It records request
bodies at `/_fake/state.syncs`. It never binds a port during unit tests.

`POST /_fake/atlas-edit` accepts `{project_uuid?, spots?, grains?, labels?}` or
`{project_uuid?, changes: <delta>}`. Explicit `project_uuid: null` edits
project-less mount elements; `mount_id` can select the mount. Offline and reset
controls remain available while offline. `/_fake/options` controls can_edit,
link_conflict, validation_errors and warnings for tests. `/_fake/legacy-grain`
turns one fixture grain into an id-null database row so adoption is exercised.
Injected spots may carry acquisition_ids; sync refuses measured deletions.
The fake implements `complete`, atomic `snapshot_incomplete` refusals, and
`/_fake/add-after-copy` to add a spot after a client opened its copy. The adopted
r2 reset displays 28 active grains for fresh-profile deletion tests. It implements keep-null fields, explicit spot detachment, unknown sample
warnings, isolated dry runs and the bulk guard. It records previews separately
at `/_fake/state.dryRuns`; neither preview revisions nor audit rows persist.
It does not implement full geometry/schema validation, all cross-project
ownership rules, real acquisition pairing or revision changes caused
by legacy Advanced calls. These still require a migrated scratch B1 service.

Unit tests cover the controller, transport/fake in process, stored-field diffs,
merge, readiness, read-only listing, opaque hashes, rebasing, snapshots,
retirement, lineage, UUID preservation, pending snapshots and Save/sync storage
key invariants. Browser specs cover wire shapes,
non-modal linking, and result-dot rings plus compact legends at 1280 × 800 and
1440 × 900. Existing stored-copy and grain-owner regression tests remain intact.
The age overlay uses a 1.5 px white inner ring and a 0.75 px dark outer ring,
with constant screen widths and zoom-adjusted radii in both themes. Flags and
selection are drawn above the result rings.

## Bridge v4 and editing (Part C)

`hosted-sync.ts` uses the bridge's exported command validator, event names,
types and normalizers. `hosted-actions.ts` adapts commands to tab sessions.
The module emits status snapshots (including checking), draft snapshots at
startup and storage changes, and `lab.atlas.synced` after confirmed syncs or
updates. Status revision is the mount revision and is null without a mount.
Identified commands are serialized and produce one result after completion.
An offline open/check is successful; failed syncs retain the draft. A conflict
opens Review differences and stays in Edit. Other-module sync notices recheck
all open tabs on the same mount.

The canvas toolbar switches between View and Edit. Its own switch asks Sync
and leave, Keep draft or Stay when changes are pending. Shell commands act
without another prompt except for the held bulk flow described below. Discard removes tracking, leaves stored project bytes
alone and retains an in-memory Undo discard action until the next open. A
small discarded-uuid index prevents those stored bytes from immediately
reappearing as legacy drafts; explicit adoption starts tracking again.

Without raw pixels, Split uses a finite line crossing the polygon twice;
Merge uses polygon boundary intersections; Outline uses the drawn polygon
without snapping. Boolean results with holes, separate pieces or point-only
contacts are refused because the grain format stores one simple ring. Brush
still needs raw pixels. New geometric shapes recompute area, centroid and
bounds. Pixel splits keep the core geometry and bubble classification.

Split gives both children new uuids, the parent number and label suffixes a/b
(larger part first), with split_from. Merge creates a new uuid with merged_from,
the lowest parent number and the default area/number label. Parents leave the
active grain list and their wire deletes mean retirement. Spots retain names,
ids and positions; reassignment to children and parent retirement share one
sync request and one undo checkpoint. A boundary spot goes to the larger part;
a spot outside both pixel-derived parts refuses the edit instead of silently
unlinking it. Artifact children retain lineage and their parent's number.

Numbers are not keys. Canvas review filtering and shell selection use grain
ids. Shell lookup accepts ids and suffix labels; a bare repeated number is
ambiguous. Canvas numbers, grain tables, area selectors, grain print labels,
CSV and GeoJSON carry suffixes. Explicit Renumber assigns consecutive numbers
in reading order and retains suffixes. Spot naming still uses the existing
numeric {grain} and running {n} pattern; splitting never renames spots. The
legacy spot handover resolves grain labels by id. Masks allocate distinct u16
values to split siblings and put the value-to-uuid mapping in grains.json;
normal unique numbers keep their previous mask values. Checks already compare
grain ids and do not require unique grain numbers.

Standalone handoffs are documented in README.md. Revision arguments are a
hint for the current-state lookup, not a request for a historical bundle.
No package installation, crate fetch, Rust build, Tauri formatting, port,
Playwright run or commit is part of this update. The new browser spec uses
an iframe host and the fake service; browser and desktop execution remain
with the supervisor.

## Part C verification

`make unit`: 175 passed, no failures or skips. `svelte-check` with the normal
ui/tsconfig.json and an empty temporary Svelte config outside the repository:
0 errors and 0 warnings. The temporary config bypasses Vite's cache write
through the shared node_modules symlink. The new Playwright spec also received
a standalone TypeScript diagnostic check; Playwright itself was not invoked.
Rust unit tests are written for launch parsing, label round-trips, explicit
renumbering, CSV labels and distinct mask ids, but were not run. Cargo.lock is
unchanged; the supervisor must resolve/build the new version 2 single-instance
plugin and rebuild native/wasm artifacts. Geometry and browser layout still
need the requested browser/app checks, and live B1 behavior is not verified.

## Files changed for Part C

- `README.md`
- `crates/grainatlas-core/src/atlas/mod.rs`
- `crates/grainatlas-core/src/checks.rs`
- `crates/grainatlas-core/src/laser/mod.rs`
- `crates/grainatlas-core/src/layout/mod.rs`
- `crates/grainatlas-core/src/layout/sheets.rs`
- `crates/grainatlas-core/src/project/pipeline.rs`
- `crates/grainatlas-core/src/segment/edit.rs`
- `crates/grainatlas-core/src/segment/mod.rs`
- `crates/grainatlas-core/src/segment/pad.rs`
- `crates/grainatlas-core/src/segment/shape.rs`
- `crates/grainatlas-core/src/spots/mod.rs`
- `crates/grainatlas-core/tests/auto_tone.rs`
- `crates/grainatlas-core/tests/segment.rs`
- `crates/grainatlas-core/tests/sheets.rs`
- `crates/grainatlas-core/tests/spots.rs`
- `docs/sync-client.md`
- `tauri/src-tauri/Cargo.toml`
- `tauri/src-tauri/src/launch.rs`
- `tauri/src-tauri/src/lib.rs`
- `ui/src/App.svelte`
- `ui/src/components/DraftList.svelte`
- `ui/src/components/LaserPanel.svelte`
- `ui/src/components/LeaveEditDialog.svelte`
- `ui/src/components/SyncChip.svelte`
- `ui/src/lib/actions.ts`
- `ui/src/lib/atlas.ts`
- `ui/src/lib/core/types.ts`
- `ui/src/lib/drafts.ts`
- `ui/src/lib/grain-finding.ts`
- `ui/src/lib/grain-lineage.ts`
- `ui/src/lib/hosted-actions.ts`
- `ui/src/lib/hosted-sync.ts`
- `ui/src/lib/laicpmsdb.ts`
- `ui/src/lib/polygon-edit.ts`
- `ui/src/lib/remote.ts`
- `ui/src/lib/shellcmd.ts`
- `ui/src/lib/spots.ts`
- `ui/src/lib/standalone-launch.ts`
- `ui/src/lib/store.svelte.ts`
- `ui/src/lib/sync-session.svelte.ts`
- `ui/src/screens/Area.svelte`
- `ui/src/screens/Canvas.svelte`
- `ui/tests/hosted-sync.spec.ts`
- `ui/unit/drafts.test.ts`
- `ui/unit/grain-lineage.test.ts`
- `ui/unit/hosted-actions.test.ts`
- `ui/unit/hosted-sync.test.ts`
- `ui/unit/polygon-edit.test.ts`
- `ui/unit/standalone-launch.test.ts`
- `ui/unit/sync-session.test.ts`


## B1 contract observations and verification

Lookup counts/hashes are mount-wide, while project elements are filtered; the
client deliberately compares hashes only within the filtered scope. Link does
not copy another project's content or make its atlas current. A name candidate
already owned by another live project can still lead to cross-project ID/label
validation errors on a later snapshot; a name match alone is not proof of shared
identity. Snapshot omissions also include other projects' elements on the mount,
which are reported but never imported into this project's editable data.

SpotIn accepts sample but SpotElement does not return it. Spot sample display
therefore remains local; the client does not infer a stored sample change from
that asymmetry. Grain reads allow null outlines, and the backfill invents no
geometry. Legacy adoption uses the local bundle's matching outline where one
exists. DateOut's preferred map entry is retained, including passes; dateOf
continues to use r.date as the preferred headline.

This update runs unit tests and svelte-check, with Vite’s config runner selected
in a temporary Svelte config to avoid writing through the shared node_modules
symlink. The regular preprocessing and ui/tsconfig.json remain in use. Browser
specs are updated but are not run. No live B1 service, browser, desktop app,
Rust build, installation, port or commit is part of this verification.

B1 regression checks: `make unit` passed 196 tests with no failures or skips.
Svelte-check passed with 0 errors and 0 warnings using the temporary config
with `vitePreprocess({style: {configLoader: "runner"}})`. The two changed
Playwright specs passed standalone strict TypeScript checking; neither was run.

Files changed for B1:

- `ui/src/lib/sync-api.ts`
- `ui/src/lib/sync.ts`
- `ui/src/lib/sync-session.svelte.ts`
- `ui/src/lib/drafts.ts`
- `ui/src/lib/hosted-actions.ts`
- `ui/src/lib/hosted-sync.ts`
- `ui/src/lib/remote.ts`
- `ui/src/lib/results.ts`
- `ui/src/components/DraftList.svelte`
- `ui/src/components/SyncChip.svelte`
- `ui/src/screens/Canvas.svelte`
- `ui/scripts/fake-laicpmsdb.mjs`
- `docs/sync-client.md`
- `ui/unit/drafts.test.ts`
- `ui/unit/hosted-actions.test.ts`
- `ui/unit/hosted-sync.test.ts`
- `ui/unit/results.test.ts`
- `ui/unit/sync-api.test.ts`
- `ui/unit/sync-session.test.ts`
- `ui/unit/sync.test.ts`
- `ui/tests/sync.spec.ts`
- `ui/tests/hosted-sync.spec.ts`

The fake starts at revision 1 with B1's publish result: three legacy id-null
grains for the spot-bearing labels, null geometry and numbers, three identified
spots with null grain_id, and no labels. A test-only reset with snapshot true
selects an already-snapshotted fixture for delta scenarios. This is explicit in
those setups; ordinary opens exercise the legacy publish state.

For a bundle without raw pixels, the split tool finds a grain by the cut's two
polygon crossings. The fixture cut at HJ-GC1_1.x + 15 µm has its midpoint outside
the concave outline, so midpoint hit testing alone missed a valid cut. Unit
coverage uses that exact cut and verifies two positive-area parts with conserved
area. The hosted browser regression checks that the line draws and both children
retain the parent lineage and spot assignment.


## B1 round-trip, View and safety follow-up

Every sync first posts `dry_run: true`. Retiring more than max(3, 10%) of
the mount’s non-retired grains, or deleting more than max(3, 10%) of its spots,
requires “This retires N of M grains and deletes K spots. Continue?” Split and
merge parents count once toward retirement. A server bulk 422 also holds the
request, using its confirmation counts when supplied. Only Continue sends
`confirm_bulk: true`, for the exact held snapshot. Later local edits remain dirty.
Cancel keeps the draft and the chip says “Needs confirmation”. Each confirmation
retry dry-runs again; changed counts require a new confirmation.

A shell-started leave-edit sync or sync-draft returns the bridge-validated
`lab.command.result` with `ok: false`, `code: 'needs-confirmation'` and
`confirm: {retireGrains, grainsTotal, deleteSpots, spotsTotal}`. It opens no
prompt. A subsequent set-mode edit or open-project mode edit immediately shows
the held confirmation. Cancel preserves the held request and draft.

Spot unlinks send `detach_grain: true`. Null geometry, endpoints and sizes are
omitted from writes; they mean keep, never erase. An absent comment keeps its
stored value; an empty string clears it. Kept-null warnings are logged with
console.warn as client bugs and omitted from visible notices. Sample refusal
warnings remain non-blocking. The acknowledged base is the canonical project
elements read after the write, only when its revision matches the reply.

This pass preserves the firstSync getter’s open/check/snapshot decision and the
vertical split-line toHaveCount(1) browser assertion. Unit, component type and
strict browser-spec TypeScript checks run without installations or ports.
Playwright, desktop and live scratch-server verification remain with the supervisor.

Follow-up verification: `make unit` passed 211 tests with no failures or skips.
Svelte-check passed with 0 errors and 0 warnings using the normal tsconfig and a
temporary `vitePreprocess({style: {configLoader: "runner"}})` config outside the
repository, avoiding Vite’s cache write through shared node_modules. Both changed
browser specs passed strict standalone TypeScript checking. `git diff --check`
is clean. Playwright and the real scratch server were not run in this pass.
