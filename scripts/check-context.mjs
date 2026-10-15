import assert from 'node:assert/strict';
import { contextUsage, utf8Bytes } from '../src/context-usage.ts';

const msg = (role, text, extra = {}) => ({ id: Math.random().toString(36), role, text, status: 'completed', ...extra });
const system = 'Bạn là Lim.';
// Budget mirrors runtime.rs input_budget: context − (num_predict + system bytes + 256).
const empty = contextUsage([], '', 4096, 2048, system);
assert.equal(empty.historyBudgetBytes, 4096 - (2048 + utf8Bytes(system) + 256));
assert.equal(empty.measured, false);

// Real Ollama counts from the latest completed turn drive the estimate.
const history = [msg('user', 'hi'), msg('assistant', 'Chào bạn', { metrics: { promptTokens: 120, outputTokens: 30 } })];
const measured = contextUsage(history, 'abc', 4096, 2048, system);
assert.equal(measured.measured, true);
assert.equal(measured.nextPromptTokens, 150 + 1);
assert.equal(measured.includedMessages, 2);
assert.equal(measured.droppedMessages, 0);

// Oldest messages are dropped first once the byte budget is spent; previews, failed replies and
// the questions they leave unanswered never count (mirrors runtime.rs conversation()).
const long = 'x'.repeat(700);
const crowded = [msg('user', long), msg('assistant', long), msg('user', 'preview', { source: 'ui_preview' }), msg('user', 'hỏi lỗi'), msg('assistant', 'lỗi', { status: 'failed' }), msg('user', long), msg('assistant', long)];
const cut = contextUsage(crowded, '', 4096, 2048, system);
assert.equal(cut.includedMessages, 2);
assert.equal(cut.droppedMessages, 2);
// A question whose answer stopped while thinking (empty) is left out, so the next draft does not
// follow another user turn.
const orphan = contextUsage([msg('user', 'viết về con bò'), msg('assistant', '', { status: 'truncated' })], 'chào', 4096, 2048, system);
assert.equal(orphan.includedMessages + orphan.droppedMessages, 0);

// A draft larger than the whole history budget is flagged.
assert.equal(contextUsage([], 'y'.repeat(5000), 4096, 2048, system).draftFits, false);
console.log('Context usage checks passed');
