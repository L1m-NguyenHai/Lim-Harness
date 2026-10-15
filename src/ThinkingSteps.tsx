import type { ThinkingStep } from "./thinking-steps";

/** Reasoning shown as a numbered timeline; each step expands to its details. */
export function ThinkingSteps({ steps, live }: { steps: ThinkingStep[]; live: boolean }) {
  return <ol className={`thinking-steps ${live ? "live" : ""}`}>
    {steps.map((step, index) => {
      const current = live && index === steps.length - 1;
      return <li key={index} className={current ? "current" : ""}>
        <span className="thinking-marker" aria-hidden="true">{index + 1}</span>
        {step.body.length
          ? <details open={current}>
              <summary>{step.title}</summary>
              <div className="thinking-body">{step.body.map((line, i) => <p key={i} className={line.startsWith("• ") ? "bullet" : ""}>{line.startsWith("• ") ? line.slice(2) : line}</p>)}</div>
            </details>
          : <div className="thinking-title">{step.title}</div>}
      </li>;
    })}
  </ol>;
}
