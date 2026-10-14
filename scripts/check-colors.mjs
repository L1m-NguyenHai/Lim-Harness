import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../src/tokens.css', import.meta.url), 'utf8');
function luminance(hex) {
  let value = hex.slice(1);
  if (value.length === 3) value = [...value].map(c => c + c).join('');
  const [r, g, b] = [0, 2, 4].map(i => Number.parseInt(value.slice(i, i + 2), 16) / 255).map(v => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
function contrast(a, b) { const x = luminance(a), y = luminance(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }
for (const block of css.matchAll(/:root[^{}]*\{([^}]+)\}/g)) {
  const colors = Object.fromEntries([...block[1].matchAll(/--([\w-]+):\s*(#[\da-f]+);/gi)].map(m => [m[1], m[2]]));
  const pairs = ['background', 'surface', 'sidebar', 'accent-soft', 'hover', 'avatar'].flatMap(bg => ['text', 'muted'].map(fg => [fg, bg]));
  pairs.push(['on-accent', 'accent']);
  for (const [foreground, background] of pairs) assert.ok(contrast(colors[foreground], colors[background]) >= 4.5, `${foreground}/${background} must meet 4.5:1`);
  console.log(`Theme contrast passed; minimum ${Math.min(...pairs.map(([fg, bg]) => contrast(colors[fg], colors[bg]))).toFixed(2)}:1`);
}
