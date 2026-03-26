# Local Rules

## Architecture
- Position: ChainBot marketing surface built with Astro and app-local frontend tooling.
- Logic: `interface/AGENTS.md` -> `land-page/AGENTS.md` -> page/layout/section composition.
- Constraints: Keep content centralized, keep UI primitives reusable, avoid dependencies on the Cargo workspace, and preserve the established warm-light + green-accent palette defined in `src/styles/global.css` unless the user explicitly asks for a rebrand.

## Members
- `src/pages/`: Route entrypoints.
- `src/layouts/`: Shared document shell and metadata.
- `src/components/sections/`: Narrative sections used by the landing page.
- `src/components/ui/`: Reusable UI primitives compatible with the chosen stack.
- `src/content/`: Structured copy and section data.
- `public/`: Static branded assets.

## Dependencies
- `src/styles/global.css`: Canonical color tokens and typography direction for the landing page.

## Review Triggers
- Change the navigation contract or outbound CTAs.
- Add a new route or major section.
- Replace shared branding tokens or visual hierarchy.
