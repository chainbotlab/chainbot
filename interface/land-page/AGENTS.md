# AGENTS.md

## Scope
- Position: Astro marketing surface for the ChainBot product.
- Owns: Product narrative, call-to-action surface, and reusable UI/content structure for the landing site.
- Excludes: Cargo workspace coupling and shared repo tooling outside this app.

## Constraints
- Keep content centralized and UI primitives reusable.
- Preserve the established warm-light and green-accent palette unless an explicit rebrand says otherwise.
- Avoid dependencies on Cargo workspace internals.

## Members
- `src/pages/`: Route-level marketing pages.
- `src/layouts/`: Shared Astro layouts.
- `src/components/sections/`: Section-level page composition.
- `src/components/ui/`: Reusable visual primitives.
- `src/content/`: Centralized copy and content payloads.
- `public/`: Static assets.

## Docs
- `src/styles/global.css`: Canonical brand, color, and typography tokens for this app.
