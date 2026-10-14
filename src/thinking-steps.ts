export type ThinkingStep = { title: string; body: string[] };

const cleanInline = (text: string) => text.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1").trim();
const firstSentence = (text: string) => {
  const flat = cleanInline(text.replace(/\s+/g, " "));
  const end = flat.search(/[.!?…:](\s|$)/);
  const sentence = end > 0 && end < 110 ? flat.slice(0, end + 1) : flat;
  return sentence.length > 110 ? `${sentence.slice(0, 107).trimEnd()}…` : sentence;
};
// Body lines: sub-bullets become "• …", blank lines dropped, markdown emphasis removed.
const bodyLines = (lines: string[]) => lines
  .map((line) => line.trim())
  .filter(Boolean)
  .map((line) => /^[*\-+]\s+/.test(line) ? `• ${cleanInline(line.replace(/^[*\-+]\s+/, ""))}` : cleanInline(line));

function fromSegment(lines: string[], marker: RegExp): ThinkingStep {
  const head = lines[0].replace(marker, "").trim();
  const heading = head.match(/^\*\*(.+?)\*\*:?\s*(.*)$/);
  if (heading) return { title: cleanInline(heading[1]).replace(/:$/, ""), body: bodyLines([heading[2], ...lines.slice(1)]) };
  const text = [head, ...lines.slice(1)].join("\n");
  const title = firstSentence(text);
  const rest = cleanInline(text.replace(/\s+/g, " ")).slice(title.replace(/…$/, "").length).trim();
  return { title, body: rest ? [rest] : [] };
}

/**
 * Splits model reasoning into readable steps: top-level numbered items first, then markdown
 * headings / bold heading lines, then paragraphs. Works on partial text while streaming.
 */
export function thinkingSteps(raw: string): ThinkingStep[] {
  const text = raw.replace(/\r\n/g, "\n").replace(/^\s*(thinking process|quá trình suy nghĩ)\s*:?\s*\n/i, "").trim();
  if (!text) return [];
  const lines = text.split("\n");
  const splitAt = (test: (line: string) => boolean, marker: RegExp) => {
    const starts = lines.map((line, index) => test(line) ? index : -1).filter((index) => index >= 0);
    if (starts.length < 2) return null;
    const steps: ThinkingStep[] = [];
    const preamble = lines.slice(0, starts[0]).join("\n").trim();
    if (preamble) steps.push(fromSegment(preamble.split("\n"), /^/));
    starts.forEach((start, i) => steps.push(fromSegment(lines.slice(start, starts[i + 1] ?? lines.length), marker)));
    return steps;
  };
  const numbered = /^\s{0,3}\d{1,2}[.)]\s+/;
  const heading = /^\s{0,3}(#{1,4}\s+|\*\*[^*]+\*\*:?\s*$)/;
  const steps = splitAt((line) => numbered.test(line), numbered)
    ?? splitAt((line) => heading.test(line), /^\s{0,3}#{1,4}\s+/);
  if (steps) return steps;
  const paragraphs = text.split(/\n\s*\n/).map((part) => part.trim()).filter(Boolean);
  return paragraphs.map((part) => fromSegment(part.split("\n"), /^/));
}
