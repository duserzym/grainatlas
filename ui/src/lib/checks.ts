// Check results and label sources live in memory, outside the saved project.
import { current, type AppState } from './store.svelte';
import { authToken } from './atlas';
import { clearMountAcquisitionLabels, mountAcquisitionLabels } from './laicpmsdb';
import { acceptedChecks, findingSignature, findingsSummary } from './check-presentation';

const pending = new WeakSet<AppState>();

export async function runChecks(app: AppState = current(), refreshLabels = false) {
  const projectId = app.project?.uuid;
  if (!projectId) return [];
  let labels = app.checkLabels;
  app.checkLabelsError = null;
  app.mountLabelsCount = null;
  if (labels === null && app.server && app.mountId) {
    const server = app.server, mount = app.mountId;
    try {
      if (refreshLabels) clearMountAcquisitionLabels(server, mount);
      labels = await mountAcquisitionLabels(server, await authToken(app), mount);
      if (app.project?.uuid !== projectId || app.server !== server || app.mountId !== mount) return [];
      app.mountLabelsCount = labels.length;
    } catch (e) {
      if (app.project?.uuid !== projectId || app.server !== server || app.mountId !== mount) return [];
      app.checkLabelsError = `Could not load acquisition labels: ${e instanceof Error ? e.message : e}`;
    }
  }
  if (app.project?.uuid !== projectId) return [];
  const project = app.fullProject();
  if (!project) return [];
  // fullProject() already snapshots project state. Copy string labels too: a file
  // list can be a Svelte proxy, which the wasm worker cannot structured-clone.
  const findings = await (await app.core).checkProject(project, labels === null ? null : [...labels]);
  if (app.project?.uuid === project.uuid) {
    app.findings = findings;
    app.checksRan = true;
  }
  return findings;
}

/** Save remembers the last accepted findings; Send and Publish always offer Review/Continue. */
export async function beforeCheckedAction(app: AppState = current(), action: 'save' | 'send' | 'publish' = 'send'): Promise<boolean> {
  if (pending.has(app)) return false;
  pending.add(app);
  try {
    const findings = await runChecks(app);
    if (!findings.length) return true;
    const projectId = app.project?.uuid;
    if (!projectId) return false;
    const signature = findingSignature(findings);
    if (action === 'save' && acceptedChecks.get(projectId) === signature) return true;
    return await new Promise<boolean>((resolve) => {
      app.checkPrompt = { summary: findingsSummary(findings), answer: (proceed) => {
        app.checkPrompt = null;
        if (proceed) acceptedChecks.set(projectId, signature);
        if (!proceed) { app.checksRequested = true; app.screen = 'canvas'; app.layout.sideCollapsed = false; }
        resolve(proceed);
      } };
    });
  } catch (e) {
    // A failed check is itself advisory; it must never prevent saving or sending.
    app.notice = `Could not run checks: ${e instanceof Error ? e.message : e}`;
    return true;
  } finally { pending.delete(app); }
}

export { parseLabels } from './check-labels';
