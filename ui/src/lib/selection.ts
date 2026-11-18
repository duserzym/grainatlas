export interface Sel { ids: string[]; anchor: string | null }
export type PickHow = 'only' | 'toggle' | 'range' | 'add';

export function pick(order: string[], sel: Sel, id: string, how: PickHow): Sel {
  if (how === 'range') {
    const a = order.indexOf(sel.anchor ?? ''), b = order.indexOf(id);
    if (a >= 0 && b >= 0) return { ids: order.slice(Math.min(a, b), Math.max(a, b) + 1), anchor: sel.anchor };
  } else if (how === 'toggle') {
    return { ids: sel.ids.includes(id) ? sel.ids.filter((s) => s !== id) : [...sel.ids, id], anchor: id };
  } else if (how === 'add') return { ids: [...new Set([...sel.ids, id])], anchor: id };
  return { ids: [id], anchor: id };
}

export function selectAll(order: string[]): Sel {
  return { ids: [...new Set(order)], anchor: order[0] ?? null };
}

export function addMany(sel: Sel, ids: string[]): Sel {
  return { ids: [...new Set([...sel.ids, ...ids])], anchor: ids.at(-1) ?? sel.anchor };
}

/** Each hit toggles once, even if a spatial query returns duplicate ids. */
export function toggleMany(sel: Sel, ids: string[]): Sel {
  const hits = new Set(ids), selected = new Set(sel.ids);
  return { ids: [...selected].filter(id => !hits.has(id)).concat([...hits].filter(id => !selected.has(id))),
    anchor: ids.at(-1) ?? sel.anchor };
}

export function clickHow(e: { metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }, inTable: boolean): PickHow {
  return e.metaKey || e.ctrlKey ? 'toggle' : e.shiftKey ? inTable ? 'range' : 'add' : 'only';
}
