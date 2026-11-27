import { expect, type Locator } from '@playwright/test';

export async function opaqueAndUnclipped(panel: Locator, background: string) {
  await expect(panel).toBeVisible();
  await expect(panel).toHaveCSS('background-color', background);
  const surface = await panel.evaluate(element => {
    const style = getComputedStyle(element);
    const box = element.getBoundingClientRect();
    const points = [
      [box.left + box.width / 2, box.top + box.height / 2],
      [box.left + 8, box.top + 8], [box.right - 8, box.top + 8],
      [box.left + 8, box.bottom - 8], [box.right - 8, box.bottom - 8],
    ];
    const onTop = points.every(([x, y]) => {
      const hit = document.elementFromPoint(x, y);
      return !!hit && element.contains(hit);
    });
    const ancestors = [];
    for (let node: Element | null = element; node; node = node.parentElement) {
      ancestors.push(getComputedStyle(node).opacity);
    }
    return {
      blur: style.getPropertyValue('backdrop-filter') || 'none',
      webkitBlur: style.getPropertyValue('-webkit-backdrop-filter') || 'none',
      ancestors,
      onTop,
      fits: box.left >= 0 && box.top >= 0 && box.right <= innerWidth && box.bottom <= innerHeight,
    };
  });
  expect(surface.blur).toBe('none');
  expect(surface.webkitBlur).toBe('none');
  expect(surface.ancestors.every(opacity => opacity === '1')).toBe(true);
  expect(surface.onTop).toBe(true);
  expect(surface.fits).toBe(true);
}

