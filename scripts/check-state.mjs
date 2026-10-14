import assert from 'node:assert/strict';
import { initialState, newSession, readState, STORAGE_KEY } from '../src/state.ts';

let raw = null;
globalThis.localStorage = { getItem: () => raw };
assert.equal(readState().state.sessions.length, 1);
assert.notEqual(newSession().id, newSession().id);
const saved = initialState();
saved.notes.push({ id: 'note-1', title: 'Quyết định', content: 'Chọn Tauri', updated: new Date().toISOString() });
raw = JSON.stringify(saved);
assert.deepEqual(readState().state, saved, 'Valid saved state survives reload');
for (const field of ['session', 'note', 'activity']) {
  const damaged = structuredClone(saved);
  if (field === 'session') damaged.sessions[0].created = 'not-a-date';
  if (field === 'note') damaged.notes[0].updated = 'not-a-date';
  if (field === 'activity') damaged.activities.push({ id: 'bad', text: 'bad', time: 'not-a-date' });
  raw = JSON.stringify(damaged);
  assert.ok(readState().error, `Reject invalid ${field} date before it reaches UI`);
  assert.equal(raw, JSON.stringify(damaged), 'Loader must not overwrite original data');
}
raw = '{malformed';
assert.ok(readState().error);
globalThis.localStorage = { getItem: () => { throw new Error('Storage unavailable'); } };
assert.ok(readState().error);
console.log(`State checks passed (${STORAGE_KEY}): reload, invalid dates, corrupt JSON, inaccessible storage.`);
