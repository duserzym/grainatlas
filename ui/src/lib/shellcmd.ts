// Hosted bridge v3 commands. Pure validation/lookup stay usable by Node's unit runner.
import { isBridgeCommand, type FocusGrainCommand, type FocusSpotCommand } from '@lab/bridge';
import type { Grain, Spot } from './core/types';
import { grainNumber } from './grain-lineage.ts';
import type { AppState } from './store.svelte';

export type ShellCommand = FocusSpotCommand | FocusGrainCommand;
const ID = /^[A-Za-z0-9][A-Za-z0-9._~-]{0,127}$/;
const isId = (x: unknown): x is string => typeof x === 'string' && ID.test(x);
const isRecord = (x: unknown): x is Record<string, unknown> => !!x && typeof x === 'object' && !Array.isArray(x);

export type ReviewColours = Record<string, Record<string, string>>;
export interface ReviewGroup { name: string; color: string; spots: string[] }
export type ReviewGroups = Record<string, ReviewGroup[]>;

/** Bridge v5.2 is optional: reject malformed groups without losing valid neighbours. */
export function reviewGroups(value: unknown): ReviewGroups {
  if (!isRecord(value)) return {};
  return Object.fromEntries(Object.entries(value)
    .filter(([project, groups]) => isId(project) && Array.isArray(groups) && groups.length <= 256).slice(0, 64)
    .map(([project, groups]) => [project, (groups as unknown[]).flatMap(group => {
      if (!isRecord(group) || typeof group.name !== 'string' || group.name.length > 200 ||
        typeof group.color !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(group.color) ||
        !Array.isArray(group.spots) || group.spots.length > 10000 || ![...group.spots].every(isId)) return [];
      return [{ name: group.name, color: group.color.toLowerCase(), spots: [...group.spots] }];
    })]));
}

/** The linked bridge predates colors; keep valid entries even beside malformed ones. */
export function reviewColours(value: unknown): ReviewColours {
  if (!isRecord(value)) return {};
  return Object.fromEntries(Object.entries(value)
    .filter(([project, spots]) => isId(project) && isRecord(spots)).slice(0, 64)
    .map(([project, spots]) => [project, Object.fromEntries(Object.entries(spots as Record<string, unknown>)
      .filter(([spot, colour]) => isId(spot) && typeof colour === 'string' && /^#[0-9a-fA-F]{6}$/.test(colour)).slice(0, 10000)
      .map(([spot, colour]) => [spot, (colour as string).toLowerCase()]))]));
}

/** Exact project key like review visibility; spot ids follow shell focus's sameId rule. */
export function reviewColourLookup(colors: ReviewColours, project: string | null | undefined): (spot: string) => string | null {
  const entries = project && Object.hasOwn(colors, project) ? colors[project] : {};
  const byId = new Map(Object.entries(entries).map(([spot, colour]) => [spot.toLowerCase(), colour]));
  return (spot) => byId.get(spot.toLowerCase()) ?? null;
}

export function commandServer(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const u = new URL(value);
    return /^https?:$/.test(u.protocol) && !!u.hostname ? value : null;
  } catch { return null; }
}

export function isShellCommand(value: unknown): value is ShellCommand {
  return isBridgeCommand(value) && (value.type === 'focus-spot' || value.type === 'focus-grain');
}

/** Id first, then area/number, then an unambiguous number (spot's area wins). */
export function findGrain(grains: Record<string, Grain[]>, grain: string, spot?: Pick<Spot, 'area_id'> | null): { area: string; grain: Grain } | null {
  const all = Object.entries(grains).flatMap(([area, gs]) => gs.map((g) => ({ area, grain: g })));
  const byId = all.find((g) => g.grain.id.toLowerCase() === grain.toLowerCase());
  if (byId) return byId;
  const parts = grain.split('/');
  if (parts.length > 2) return null;
  const number = parts.at(-1)!;

  const matches = all.filter((g) => grainNumber(g.grain) === number &&
    (parts.length === 2 ? g.area === parts[0] : !spot || g.area === spot.area_id));
  return matches.length === 1 ? matches[0] : null;
}

/** Runs only once the requested project is open; also used when its package arrives later. */
export async function applyShellFocus(app: AppState, cmd: ShellCommand) {
  app.flash = null;
  const { stitch } = await import('./actions');
  const { sameId } = await import('./deeplink');
  const spot = app.spots.find((s) => sameId(s.id, cmd.spot));
  if (cmd.type === 'focus-spot') {
    if (!spot) {
      app.notice = `Spot ${cmd.spot} is not in project ${app.project?.name || cmd.project}.`;
      return;
    }
    if (!app.stitched[spot.area_id] && !(await stitch(spot.area_id, app))) return;
    app.areaId = spot.area_id;
    app.selection = [spot.id];
    app.shellSelection = { project: app.project?.uuid ?? cmd.project, spot: spot.id };
    app.focusGrain = null;
    app.focusSpot = spot.id;
    if (cmd.flash !== false) app.flash = { kind: 'spot', id: spot.id, area: spot.area_id };
  } else {
    const hit = findGrain(app.grains, cmd.grain, spot);
    if (!hit) {
      app.notice = `Grain ${cmd.grain} is not in project ${app.project?.name || cmd.project} or its number is ambiguous.`;
      return;
    }
    if (!app.stitched[hit.area] && !(await stitch(hit.area, app))) return;
    app.areaId = hit.area;
    app.selection = [];
    app.focusSpot = null;
    app.focusGrain = { area: hit.area, id: hit.grain.id, flash: cmd.flash !== false };
  }
  app.screen = 'canvas';
}

export async function handleShellCommand(value: unknown) {
  const { isHosted } = await import('@lab/bridge');
  if (!isHosted()) return;
  if (isBridgeCommand(value) && value.type === 'set-presentation') {
    const { presentation } = await import('./presentation.ts');
    await presentation.command(value, true);
    return;
  }
  const { reviewSpots } = await import('./store.svelte');
  if (isBridgeCommand(value) && value.type === 'review-spots') {
    reviewSpots.projects = value.projects;
    reviewSpots.colors = reviewColours('colors' in value ? value.colors : undefined);
    reviewSpots.groups = reviewGroups('groups' in value ? value.groups : undefined);
    if (value.focus) await handleShellCommand({ ...value.focus, type: 'focus-spot' });
    return;
  }
  if (isBridgeCommand(value) && value.type !== 'focus-spot' && value.type !== 'focus-grain' && value.type !== 'review-spots') {
    const { hostedSync } = await import('./hosted-sync');
    const { runHostedCommand } = await import('./hosted-actions');
    await hostedSync.command(value, runHostedCommand);
    return;
  }
  if (!isShellCommand(value)) return;
  const cmd = value;
  const { sameId, packageNameFor } = await import('./deeplink');
  const { targetTab } = await import('./actions');
  const app = targetTab(cmd.project);
  app.pendingCommand = cmd;
  app.deepLink = { project: cmd.project, spot: cmd.spot ?? null, name: packageNameFor(cmd.project) };
  if (!sameId(app.project?.uuid, cmd.project)) {
    const requestedServer = commandServer(cmd.server);
    if (requestedServer) app.serverOverride = requestedServer;
    const server = requestedServer ?? app.server;
    if (server) {
      const { openFromServer } = await import('./remote');
      if (await openFromServer(server, cmd.project, null, null, null, app)) app.error = null;
    }
    if (!sameId(app.project?.uuid, cmd.project)) {
      app.screen = 'open';
      app.notice = app.deepLink.name ? `Open the SEM package ${app.deepLink.name} to continue.` : `Open the SEM package for project ${cmd.project}, then load its project.json to continue.`;
      return;
    }
  }
  app.pendingCommand = null;
  app.deepLink = null;
  await applyShellFocus(app, cmd);
}
