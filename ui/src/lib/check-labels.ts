/** Plain lines or CSV with a label/name column, including quoted fields. */
export function parseLabels(text: string): string[] {
  const rows: string[][] = []; let row: string[] = []; let cell = ''; let quoted = false;
  const input = text.replace(/^\uFEFF/, '').replace(/\r\n?/g, '\n');
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (c === '"') {
      if (quoted && input[i+1] === '"') { cell += '"'; i++; } else quoted = !quoted;
    } else if (!quoted && (c === ',' || c === '\n')) {
      row.push(cell.trim()); cell = '';
      if (c === '\n') { rows.push(row); row = []; }
    } else cell += c;
  }
  row.push(cell.trim()); rows.push(row);
  const header = rows[0]?.findIndex((v) => /^(label|name)$/i.test(v)) ?? -1;
  return [...new Set((header >= 0 ? rows.slice(1) : rows).map((r) => header >= 0 ? r[header] : r.join(',')).filter(Boolean))];
}
