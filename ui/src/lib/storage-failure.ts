/** The app's own copy of a project, or its sync draft, could not be written (storage full or
 * unavailable). Until a later write succeeds the tab shows a banner that cannot be dismissed. */
export function storageFullMessage(name: string): string {
  return `${name} could not be kept in this app: its storage is full. Use Save project to keep your changes in a file.`;
}

type StorageOwner = { project: { uuid: string; name?: string; package?: { name?: string } } | null; storageWarning: string | null };
type Write = 'project' | 'draft';
// Memory only. A successful draft must not clear a still-failing project copy (or the reverse).
const failures = new WeakMap<StorageOwner, Map<string, Set<Write>>>();

export function reportStorageWrite(app: StorageOwner, write: Write, succeeded: boolean): void {
  const project = app.project;
  if (!project) return;
  let projects = failures.get(app);
  if (!projects) failures.set(app, projects = new Map());
  const failed = projects.get(project.uuid) ?? new Set<Write>();
  if (succeeded) failed.delete(write);
  else failed.add(write);
  if (failed.size) projects.set(project.uuid, failed);
  else projects.delete(project.uuid);
  app.storageWarning = failed.size ? storageFullMessage(project.name || project.package?.name || 'This project') : null;
}
