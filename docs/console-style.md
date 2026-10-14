# Console visual style — 14/10/2026

Reference supplied by the user: https://www.figma.com/design/pe8nAegJz8dOxmt9ybKwlr/Untitled?node-id=1-2

The Stitch/Figma design is a style reference, not a request for exact screen reproduction. Adapted near-black surfaces (#030406/#080b0f), cyan accent (#38bdf8), fine dividers (#21262d), monospace type, dense session explorer, left-aligned message stream, docked composer, and a compact right inference inspector. Existing Lim vector logo and Lucide icons are retained. No mock telemetry, sandbox capability or hosted model values from the prototype are introduced.

Implementation: tokens.css defines the shared light/dark palette; console.css adapts existing component styling. Existing preferences migrate to dark once (`lim-console-style-v1`); light/dark switching continues to persist. Topbar model, temperature and context chips use actual selected settings. Chat, floating tools, memory and inference handlers remain in place.

Font stack uses local Cascadia Code / JetBrains Mono / Consolas; no remote fonts or new dependencies. After the council recommendations, normal messages and system prompt use 14px, important hints use 12px. Narrow viewports retain the existing sidebar rail and inference overlay.
