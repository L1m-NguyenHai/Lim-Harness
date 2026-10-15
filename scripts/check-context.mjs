import assert from 'node:assert/strict';
import { contextUsage, estimateTokens } from '../src/context-usage.ts';

const msg = (role, text, extra = {}) => ({ id: Math.random().toString(36).slice(2), role, text, status: 'completed', ...extra });
const system = 'Bạn là Lim.';
// Input budget mirrors runtime.rs input_budget: context − output − system tokens − 256.
const empty = contextUsage({ messages: [] }, '', 4096, 2048, system);
assert.equal(empty.inputBudget, 4096 - 2048 - 256 - estimateTokens(system));
assert.equal(empty.nextPromptTokens, estimateTokens(system));

// Answered turns count; a question left unanswered (stopped while thinking) does not.
const orphan = contextUsage({ messages: [msg('user', 'viết về con bò'), msg('assistant', '', { status: 'truncated' })] }, 'chào', 4096, 2048, system);
assert.equal(orphan.includedMessages + orphan.droppedMessages, 0);
assert.equal(orphan.draftTokens, estimateTokens('chào'));

// Messages covered by the summary are replaced by the summary block.
const first = msg('user', 'cũ'); const firstAnswer = msg('assistant', 'trả lời cũ');
const withSummary = contextUsage({ messages: [first, firstAnswer, msg('user', 'mới'), msg('assistant', 'trả lời mới')], summary: 'Người dùng hỏi điều cũ.', summaryUntil: firstAnswer.id }, '', 4096, 2048, system);
assert.equal(withSummary.summarizedMessages, 2);
assert.equal(withSummary.includedMessages, 2);
assert.ok(withSummary.summaryTokens > 0);

// Long conversation: over the 75% threshold, newest turns fit first, oldest are dropped.
const long = 'x'.repeat(1500); // ~504 tokens each
const turns = Array.from({ length: 4 }, () => [msg('user', long), msg('assistant', long)]).flat();
const full = contextUsage({ messages: turns }, '', 4096, 2048, system);
assert.equal(full.overThreshold, true);
assert.equal(full.canCompact, true);
assert.equal(full.includedMessages, 3);
assert.equal(full.droppedMessages, 5);
// A bigger window holds everything and stays under the threshold.
assert.equal(contextUsage({ messages: turns }, '', 32768, 2048, system).overThreshold, false);

// A draft larger than the whole input budget is flagged.
assert.equal(contextUsage({ messages: [] }, 'y'.repeat(9000), 4096, 2048, system).draftFits, false);
console.log('Context usage checks passed');
