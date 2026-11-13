# Database sync client (B2–B4)

The client follows the B1 deltas supplied on 12 November 2026. The transport is
still provisional: `ui/src/lib/sync-api.ts` owns its URLs, authentication, wire
types and mappings. No live service or real project data was used.

## Elements and revisions

A revision belongs to a mount. Several project uuids may share it. The client
never adopts a different project uuid automatically. A package-name match is
available only through the Not in the database chip. Its popover offers Link
and Not this one; it never opens itself or blocks the page. The draft remembers
the lookup and dismissed offers. An explicit link stores `linked_uuid` without
rewriting the local project's uuid.

`GET /v1/atlas/projects/{uuid}/elements` returns `{mount_id, revision, spots,
grains, labels, hashes}` for that uuid and project-less elements on its mount.
It supplies current elements only. A saved older base plus
`GET /v1/mounts/{id}/changes?since=N` supplies three-way review. Legacy drafts
without a base use current `/elements` as their initial base and keep their
local edits. History entries belonging to other project uuids never enter the
project's merge.

Only these stored fields enter element equality, ordinary upserts and hashes:

| Kind | Fields |
| --- | --- |
| Spot | id, name, area_id, grain_id, x_um, y_um, x2_um, y2_um, diameter_um, kind, comment |
| Grain | id, area_id, number, label, polygon_um, centroid_um, area_um2, sample, artifact |
| Label | id, area_id, x_um, y_um, text |

A missing grain label defaults to `<area_id>/<number>`. Grain sample comes from
its area when the local grain has no explicit sample. Null sample and artifact
values are normalized. Grains are flat on the wire, grouped by area in the UI.
The bundle or existing local copy supplies UI-only measurements; narrowed
server geometry supplies bounds and fallback measurements for new grains.

`server_id`, `project_uuid`, `acquisition_ids`, retirement and lineage readbacks,
children, and UI-only metadata never count as local edits. Retired grains are
excluded from active elements. Acquisition links still prevent the unsafe Both
choice for a conflicting spot.

The first registration of a project sends `mode: "snapshot"` and every local
element. Subsequent syncs send `mode: "delta"` with only changed elements.
Snapshot omissions are preserved by the server and reported in
`not_in_snapshot`. The client incorporates an acknowledged snapshot's omissions
without overwriting edits made while it was submitted. Every grain delete
retires the grain, preserving server identity. Split/merge additions carry
`split_from` / `merged_from`; every spot on a parent must be reassigned in the
same request. The current grain editor's identity policy remains Part C work.

The 200 reply contains `mount_id`, `project_uuid`, `mode`, `previous_revision`,
`revision`, `applied`, per-kind `ids`, `acquisitions_linked`, `warnings`,
`rebased_over`, and `not_in_snapshot`. `ids` maps stable client uuids to database
server ids; it does not rename the local elements or their references. A stale
base caused solely by other projects can succeed with `rebased_over`.

A 409 reply contains `detail`, `base_revision`, `current_revision` and
`changes: [{revision, kind, id, op, data, project_uuid, at, by}]`. The transport
converts that ordered event list to the three-way input, including deletion and
retirement. Review lists every change and requires explicit choices for
collisions. Both can duplicate annotations and differently named spots without
acquisition links. Geometry and deletion/edit conflicts require Mine or Theirs.
Another concurrent write produces another review.

## Hashes

B1 hashes each non-retired narrowed element with SHA-256 over Python
`json.dumps(el, sort_keys=True, separators=(",", ":"))`. A kind hash is SHA-256
of `<id> <element sha>\n` lines sorted by id. Empty kinds hash the empty string.

The browser never compares its own hash to a server hash. Python and JavaScript
float spellings differ. The client saves opaque server `base_hashes` at an
acknowledged base and compares them only to later server hashes. Local changes
come from stored-field element diffs. Consequently another project's mount
revision does not make unchanged elements look newer. Older draft records
without hashes conservatively fall back to the revision until acknowledged.
A later server revision observed after a POST is not silently acknowledged.

`sync-hash.ts` provides canonical equality and a test/fake hash helper. The fake
uses the B1 per-element/line construction and ASCII escaping, with JavaScript
number spelling; it is not a Python float serialization reference.

## Storage and readiness

`grainatlas.draft.<uuid>` contains uuid, name, server, mount, base_revision,
base elements, base_hashes, synced, updated_at, changes, pending, pending_at, source_keys,
linked_uuid, link_offered, link_candidates, pixels_base and pixels_changed.
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
A failed explicit sync can retry after a successful check; concurrent changes
remain behind Review differences. Read-only server viewing preserves an
existing editable draft's base.

## Fake service and verification

`ui/scripts/fake-laicpmsdb.mjs` seeds isolated namespaces from
`target/atlas-fixture`. It models mount revisions, current elements, change
history, snapshot/delta sync, retirement, lineage spot reassignment, per-kind
server ids, 200/409 shapes and rebasing over other projects. It records request
bodies at `/_fake/state.syncs`. It never binds a port during unit tests.

`POST /_fake/atlas-edit` accepts `{project_uuid?, spots?, grains?, labels?}` or
`{project_uuid?, changes: <delta>}`. Explicit `project_uuid: null` edits
project-less mount elements; `mount_id` can select the mount. Offline and reset
controls remain available while offline. The fake does not model protected
analysis deletion, full geometry validation, permissions beyond its test token,
or revision changes caused by legacy Advanced calls.

Unit tests cover the controller, transport/fake in process, stored-field diffs,
merge, readiness, read-only listing, opaque hashes, rebasing, snapshots,
retirement, lineage, UUID preservation, pending snapshots and Save/sync storage
key invariants. Browser specs cover wire shapes,
non-modal linking, and result-dot rings plus compact legends at 1280 × 800 and
1440 × 900. Existing stored-copy and grain-owner regression tests remain intact.
The age overlay uses a 1.5 px white inner ring and a 0.75 px dark outer ring,
with constant screen widths and zoom-adjusted radii in both themes. Flags and
selection are drawn above the result rings.

The installed bridge does not expose the new bridge v4 atlas helpers. No
sibling repository or installed dependency was changed. This update does not
claim bridge v4 integration or live B1 compatibility testing. Browser and
desktop checks remain with the supervisor; no Playwright run, port, install,
Rust formatting or commit is part of this update.

## Files changed in this follow-up

- `docs/sync-client.md`
- `ui/scripts/fake-laicpmsdb.mjs`
- `ui/src/App.svelte`
- `ui/src/components/DraftList.svelte`, `ui/src/components/SyncChip.svelte`
- `ui/src/lib/core/types.ts`, `ui/src/lib/drafts.ts`, `ui/src/lib/remote.ts`
- `ui/src/lib/sync.ts`, `ui/src/lib/sync-api.ts`, `ui/src/lib/sync-hash.ts`, `ui/src/lib/sync-session.svelte.ts`
- `ui/src/screens/Canvas.svelte`
- `ui/tests/sync.spec.ts`, `ui/tests/results.spec.ts`
- `ui/unit/drafts.test.ts`, `ui/unit/sync.test.ts`, `ui/unit/sync-api.test.ts`, `ui/unit/sync-hash.test.ts`, `ui/unit/sync-session.test.ts`
