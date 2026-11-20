# Plucked for TIMS

Link the project to a registered mount in the database first. On the Canvas,
choose Pluck (P) and click a grain or LA spot, or right-click and choose
Pluck for TIMS. Shift-click adds grains to one fraction. All grains must come
from the same sample; choosing two spots on one grain keeps the first spot.

Choose a capsule set, batch and free capsule, then check the suggested fraction
name and mineral. The last capsule set used on this computer is the default.
Existing batches are listed newest first. Taken capsules cannot be selected.
A fragment needs a note, which applies to every grain in the new fraction.
The TIMS section in the grain or spot panel offers editing and removal.
Removal keeps the database history.

Canvas badges show the capsule and fraction, for example `K3 · z3`, or just
`z3` without a batch. A dagger (`†`) marks a fragment. A dashed badge means the
pluck is waiting to send; a failed write uses the danger colour and can be
edited or discarded. Plucking never changes grain or spot identities, names,
mount sync content, or the project file.

Plucks are saved on this computer before sending. Offline work is queued for
the same account, server and mount. Signing out hides it and keeps the unsent
work. A grain that has not reached the database waits for mount sync; unrelated
plucks can still send. Changes to the same batch, fraction or source keep their
order.

In Layout (the printout screen), choose Pick list. It is available when this
mount has plucked grains, including pending ones. Filter by capsule set or
batch, set the crop margin in µm and the square crop size on paper in mm, and
choose paper and a title. Preview makes one row per fraction with its capsule,
sample, grain and LA spot labels, area, fragment note, notes, grain crops and
a tick box. Multiple grains share one row. Crops use the canvas's local
pyramid tiles with the displayed tone and image edits. Pending rows say “not yet in
the database”. Each capsule set and batch combination has its own group of
pages. Each page identifies the mount, set, batch date and page count.
Print… and Save PDF use the same paths as the other print styles. A project
opened from the database needs its editable copy's image download to finish
before local crops can be made.

Import pick list… is optional in Open and next to the print styles. Paste
picking notes or choose a `.md` or `.txt` file. Select the lab when several are
available. First and last line are optional, inclusive line numbers. An optional
start date is used only for picks without a date heading.

Preview asks the database to resolve each line without writing. It shows
resolved samples, mounts, spots, grains, capsules and fractions, with refused
lines and their reasons. Lines for the open mount are highlighted. Ignored
lines are hidden until Show ignored lines is selected. The batch list says
which batches would be created or reused. Fix the notes or narrow the range
and preview again if any selected line is refused. Already imported lines are
skipped.

Import is available only with at least one accepted pick and no selected
refusals. Check the fraction and batch counts, then choose Confirm import to
write to the database. A retry of the same request keeps its import key. A
refusal at import writes nothing and shows the updated plan. Success reports
what was created and refreshes this mount's badges. Import needs the database;
offline, the dialog keeps the notes while it remains open or is closed and
reopened on the same screen. Notes are never stored in the project file.
