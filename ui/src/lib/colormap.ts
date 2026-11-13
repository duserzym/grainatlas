// sRGB stops, shared by the results layer and its legend.
export type RGB = readonly [number, number, number];
const VIRIDIS: readonly RGB[] = [[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]];

/** CIELAB L*: sRGB → linear RGB → XYZ (D65, Yn = 1) → L*. Only Y enters L*. */
export function lightness(rgb: RGB): number {
  const [r, g, b] = rgb.map((v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  const y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
  const d = 6 / 29;
  return 116 * (y > d ** 3 ? Math.cbrt(y) : y / (3 * d ** 2) + 4 / 29) - 16;
}

export function isMonotonicLightness(stops: readonly RGB[]): boolean {
  if (stops.length < 2 || stops.some((c) => c.some((v) => !Number.isFinite(v) || v < 0 || v > 255))) return false;
  const ls = stops.map(lightness);
  return ls.every((l, i) => !i || l >= ls[i - 1]) || ls.every((l, i) => !i || l <= ls[i - 1]);
}

/** Ages always start at the lighter end. Diverging and constant maps cannot encode ages. */
export function orientForAge(stops: readonly RGB[]): readonly RGB[] {
  if (!isMonotonicLightness(stops) || lightness(stops[0]) === lightness(stops[stops.length - 1])) throw new Error('Age colormaps need monotonic lightness');
  return lightness(stops[0]) > lightness(stops[stops.length - 1]) ? [...stops] : [...stops].reverse();
}

export const AGE_COLORMAPS = [{ id: 'viridis', stops: orientForAge(VIRIDIS) }] as const;

function colorAt(t: number, stops: readonly RGB[], rounded = true): string {
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0)) * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(x));
  const f = x - i;
  const c = stops[i].map((a, k) => {
    const v = a + (stops[i + 1][k] - a) * f;
    return rounded ? Math.round(v) : v;
  });
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

const gradient = (stops: readonly RGB[]) => `linear-gradient(90deg, ${stops.map((c) => `rgb(${c.join(',')})`).join(', ')})`;
// Keep fractional channels for ages: integer rounding can introduce small L* reversals.
export const ageColor = (t: number, stops: readonly RGB[] = AGE_COLORMAPS[0].stops) => colorAt(t, orientForAge(stops), false);
export const ageGradient = (stops: readonly RGB[] = AGE_COLORMAPS[0].stops) => gradient(orientForAge(stops));
export const AGE_RAMP_CSS = ageGradient();

// Non-age metrics retain the original viridis direction and interpolation.
export const rampColor = (t: number) => colorAt(t, VIRIDIS);
export const RAMP_CSS = gradient(VIRIDIS);
