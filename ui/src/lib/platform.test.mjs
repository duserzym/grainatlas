import assert from 'node:assert/strict';
import test from 'node:test';
import { primaryModifier } from './platform.ts';

test('modifier hints use Cmd on Mac and Ctrl on Windows and Linux', () => {
  for (const platform of ['MacIntel', 'MacPPC']) assert.equal(primaryModifier(platform), 'Cmd');
  for (const platform of ['Win32', 'Win64', 'Linux x86_64', '']) assert.equal(primaryModifier(platform), 'Ctrl');
});
