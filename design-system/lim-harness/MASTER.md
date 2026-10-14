# Lim Harness — Frontend design system v0.6

## Direction

Chat-first desktop harness inspired by Odysseus: one compact left sidebar and a broad chat canvas. Neutral light surfaces, charcoal dark surfaces and restrained coral accent. Vietnamese copy and Tauri native window chrome. Keep chat, memory, modules and settings. Session history stays in the sidebar and search.

Verified reference: `odysseus/assets/branding/odysseus-browser.jpg`, `odysseus/static/index.html` (sidebar, welcome and composer) and `odysseus/static/css/01-agent-chat.css` (sidebar, history and compact-height rules). This is a visual/interaction reference; no Odysseus source code or assets are copied into the shipped UI. Odysseus remains unchanged.

UI UX Pro Max quick reference informs accessibility and interaction. Its CLI did not return a verified match for the targeted disclosure query after one retry; the simplified layout is manually based on the actual Odysseus reference and the user’s request.

## Tokens

Implementation source: `src/tokens.css`. Light/dark semantic colors for background, surface, sidebar, text, muted, border, accent and focus. Light primary #983e4c with white text, background #f7f7f8. Dark primary #e6a0a8 with #24272e text, background #262a33. Checked semantic text pairs: minimum 5.07:1 light and 5.97:1 dark.

System font stack: Segoe UI, Inter if installed, system-ui. No separate display font, font CDN or remote imagery. Normal body text 14px, supporting text 11–13px and restrained 24–28px headings.

## Layout and interaction

- Desktop: 204px navigation and flexible conversation; 52px icon rail when collapsed or at phone width. Header/footer stay reachable and session history scrolls independently. Navigation controls are 32px tall, history rows 30px and header 38px.
- Empty chat: small logo/wordmark and one-line welcome above a 720px composer. Composer starts at about 88px tall with a single-line input; the user can resize the text area vertically. Active chat: transcript expands and composer stays at the bottom. Short viewports use safe alignment/scrolling.
- Remove permanent right context panel, suggestion cards, slogan, fake workspace/account UI, redundant Sessions page and frontend-only Activity page. Module choices use a compact list and a composer shortcut.
- Lucide SVG icons, consistent 1.5–2px strokes. No emoji navigation.
- Native dialog handles search and note editing with keyboard focus and Escape. Visible focus rings; controls use buttons/labels, switches expose state.
- Ctrl/Cmd K search; Ctrl/Cmd N new session; Enter submits and Shift Enter inserts a newline. IME composition does not submit.
- Motion is limited to 150–160ms feedback and respects reduced-motion.
- States: empty data, missing provider/vault, storage errors, clipboard/file errors and local confirmation. Never display a fake backend connection or model result.

## Data boundary

UI prototype persists sessions, notes, module selections and appearance in localStorage. Notes are not vault files; module choices do not execute a model. Local responses are visibly marked as UI examples. File attachments support local text/Markdown up to 1 MB. No credentials are stored.

Current native core uses SQLite for conversation transcripts and Ollama for streaming; old preview responses remain labeled and excluded from model input. Vault notes use a separate read-only native adapter. Local note drafts and appearance still use localStorage.

## Cookbook

Sidebar tools open independent floating windows inside the app over the main chat. Title bars support drag, minimize, maximize/restore and close; resize handles support pointer and keyboard. Minimized tools remain in a bottom dock. Opening another tool keeps existing windows and jobs alive; selecting chat minimizes open tools. Multiple windows use focus order for stacking, with no modal backdrop.

Cookbook follows the user’s screenshot and the actual Odysseus window at localhost:7000: internal Active, Launch, Download, Dependencies and Settings tabs; Direct Download and Scan sections, hardware chips and a compact sortable model table. Source reference only, no copied code. Reuse neutral/charcoal surfaces, coral actions and system fonts. Keep actual catalog/installed data and fit rationale; do not copy sample speed, score or fit claims. Download-only and load are distinct operations.

Catalog now has 67 verified tags from 20 families in a bundled snapshot, plus bounded online refresh/cache. Collapse Direct Download by default to prioritize the long model table. Add Auto/RAM/GPU assessment, capability and fit filters. FIT SCORE is explained as memory compatibility, not quality; SPEED shows observed Ollama eval throughput for the selected context or a dash. Keep unknown hardware in browser preview explicit. Expose metadata sources, timestamps, declared assumptions and embedding restrictions in row details.

Use explicit state text, layer-specific progress, cancellation and retry. Preserve operation while navigating to other pages. No made-up hardware in browser preview, benchmark scores or successful load state inferred from a button click. Responsive grid uses three/two/one columns. UI UX Pro Max compact-control guidance informs native button semantics and visible selection state.

## Logo

Original geometric connection mark: two open bracket shapes around a shared diamond core. No letter initial. The mark represents the harness connecting capabilities around a model/core.

- `src/Logo.tsx`: inline vector with currentColor for adaptive UI themes; used in sidebar, welcome and assistant avatar.
- `public/logo.svg`: transparent standalone vector.
- `public/app-icon.svg`: dark rounded tile for app/browser identity.
- `src-tauri/icons/icon.png` and `icon.ico`: generated from app-icon.svg using the Tauri icon CLI; generation previews in `artifacts/icon-build/`.
- Keep the same geometry across assets; decorative instances are hidden from screen readers, while enclosing controls retain accessible names.
