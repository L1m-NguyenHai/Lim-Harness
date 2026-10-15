import assert from 'node:assert/strict';
import { thinkingSteps } from '../src/thinking-steps.ts';

// Numbered list with bold headings and nested bullets (qwen-style "Thinking Process").
const numbered = thinkingSteps(`Thinking Process:

1.  **Analyze the Request:**
    *   Task: Write a short essay.
    *   Constraint: Vietnamese.

2.  **Review Constraints & Safety:**
    *   Benign request.
3.  Draft the answer in **Vietnamese** now.`);
assert.equal(numbered.length, 3);
assert.equal(numbered[0].title, 'Analyze the Request');
assert.deepEqual(numbered[0].body, ['• Task: Write a short essay.', '• Constraint: Vietnamese.']);
assert.equal(numbered[2].title, 'Draft the answer in Vietnamese now.');

// Plain paragraphs become one step each, titled by their first sentence.
const paragraphs = thinkingSteps('The user greets me. They wrote "chào".\n\nI should reply politely in Vietnamese.');
assert.equal(paragraphs.length, 2);
assert.equal(paragraphs[0].title, 'The user greets me.');
assert.deepEqual(paragraphs[0].body, ['They wrote "chào".']);
assert.deepEqual(paragraphs[1].body, []);

// One numbered item followed by paragraphs (real qwen output): no "1." title, bullets kept.
const mixed = thinkingSteps(`Thinking Process:

1.  **Analyze the Request:**
    *   User says "chào".
    *   Context: earlier prompts.

Actually, looking at the raw text again.
    *   Two user turns in a row.`);
assert.equal(mixed[0].title, 'Analyze the Request');
assert.deepEqual(mixed[0].body, ['• User says "chào".', '• Context: earlier prompts.']);
assert.equal(mixed[1].title, 'Actually, looking at the raw text again.');
assert.deepEqual(mixed[1].body, ['• Two user turns in a row.']);
// A number alone on its line takes the next line as the heading.
assert.equal(thinkingSteps('1.\n**Plan:**\n* a\n\n2.\n**Write:**')[0].title, 'Plan');

// Partial text while streaming still yields a step; empty text yields none.
assert.equal(thinkingSteps('Đang phân tích câu hỏi').length, 1);
assert.equal(thinkingSteps('  ').length, 0);
console.log('Thinking step checks passed');
