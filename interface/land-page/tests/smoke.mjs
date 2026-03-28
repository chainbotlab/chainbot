import { access, readFile } from "node:fs/promises";
import assert from "node:assert/strict";
import { resolve } from "node:path";

const rootPage = resolve(process.cwd(), "src/pages/index.astro");
const zhPage = resolve(process.cwd(), "src/pages/zh/index.astro");
const frPage = resolve(process.cwd(), "src/pages/fr/index.astro");

await access(rootPage);
await access(zhPage);
await access(frPage);

const pageSource = await readFile(rootPage, "utf8");
const zhSource = await readFile(zhPage, "utf8");
const frSource = await readFile(frPage, "utf8");

assert.match(pageSource, /<LandingPage locale="en" \/>/, "root page should render English landing content");
assert.match(zhSource, /<LandingPage locale="zh" \/>/, "zh route should render Chinese landing content");
assert.match(frSource, /<LandingPage locale="fr" \/>/, "fr route should render French landing content");

const landingSource = await readFile(resolve(process.cwd(), "src/components/LandingPage.astro"), "utf8");
const headerSource = await readFile(resolve(process.cwd(), "src/components/sections/LandingHeader.astro"), "utf8");
const heroSource = await readFile(resolve(process.cwd(), "src/components/sections/HeroStage.astro"), "utf8");
const footerCtaSource = await readFile(resolve(process.cwd(), "src/components/sections/FooterCta.astro"), "utf8");
const tickerSource = await readFile(resolve(process.cwd(), "src/components/sections/IntegrationsTicker.astro"), "utf8");
const brandMarqueeSource = await readFile(resolve(process.cwd(), "src/components/sections/BrandMarquee.tsx"), "utf8");
const marqueeSource = await readFile(resolve(process.cwd(), "src/components/ui/marquee.tsx"), "utf8");
const contentSource = await readFile(resolve(process.cwd(), "src/content/site.ts"), "utf8");

assert.match(landingSource, /<LandingHeader/, "landing page should compose a dedicated header section");
assert.match(landingSource, /<ArchitectureShowcase/, "landing page should compose the architecture section");
assert.match(headerSource, /localeHref\[locale as Locale\]/, "language switcher should use centralized locale routes");
assert.match(heroSource, /content\.install\.title/, "hero stage should render install card content from centralized data");
assert.match(heroSource, /href=\{docsUrl\}/, "hero documentation link should navigate directly to docs URL");
assert.match(heroSource, /data-copy-text=\{content\.install\.primaryCommand\}/, "hero copy button should expose copy text");
assert.match(footerCtaSource, /data-copy-text=\{content\.cta\.command\}/, "footer copy button should expose copy text");
assert.match(landingSource, /data-copy-status/, "landing page should expose a live region for copy feedback");
assert.match(tickerSource, /<BrandMarquee ariaLabel=\{content\.ariaLabel\} items=\{content\.items\} \/>/, "integrations section should render the shared brand marquee from localized content");
assert.match(brandMarqueeSource, /role="group"/, "brand marquee should expose a semantic group wrapper");
assert.match(contentSource, /eyebrow: "Built for AI-native operators"/, "integrations content should be localized from centralized data");
assert.match(contentSource, /eyebrow: "面向 AI 原生操作员"/, "Chinese integrations content should be localized");
assert.match(brandMarqueeSource, /motion-reduce:flex/, "brand marquee should expose a reduced-motion fallback");
assert.match(brandMarqueeSource, /pauseOnHover/, "brand marquee should pause on hover for readability");
assert.match(marqueeSource, /aria-hidden=\{i > 0 \? true : undefined\}/, "repeated marquee tracks should be hidden from assistive technology");
assert.match(contentSource, /ChainBot CLI/, "site content should define ChainBot branding");
assert.match(contentSource, /Claude Code/, "integrations ticker should include Claude Code");
assert.match(contentSource, /Codex/, "integrations ticker should include Codex");
assert.match(contentSource, /Kimi CLI/, "integrations ticker should include Kimi CLI");
assert.match(contentSource, /OpenClaw/, "integrations ticker should include OpenClaw");
assert.match(contentSource, /OpenCode/, "integrations ticker should include OpenCode");
assert.match(contentSource, /docs\.chainbot\.dev/, "site content should centralize docs URL management");
assert.match(contentSource, /© 2026 ChainBot/, "site content should use the 2026 footer year");
assert.match(contentSource, /copiedLabel/, "copy feedback should live in localized content");
assert.match(contentSource, /copyFailedLabel/, "copy failure feedback should live in localized content");
assert.doesNotMatch(landingSource, /www\.chainbot\.dev/, "landing page footer should use the short chainbot.dev URL only");

console.log("Landing page smoke checks passed.");
