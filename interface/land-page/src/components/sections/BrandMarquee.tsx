import claudeCodeIcon from "@lobehub/icons-static-svg/icons/claudecode-color.svg?url";
import codexIcon from "@lobehub/icons-static-svg/icons/codex-color.svg?url";
import kimiIcon from "@lobehub/icons-static-svg/icons/kimi-color.svg?url";
import openClawIcon from "@lobehub/icons-static-svg/icons/openclaw-color.svg?url";
import openCodeIcon from "@lobehub/icons-static-svg/icons/opencode.svg?url";

import type { IntegrationItem } from "@/content/site";
import { cn } from "@/lib/utils";
import { Marquee } from "@/components/ui/marquee";

type BrandDefinition = {
  iconSrc?: string;
  accentClassName: string;
};

const brandDefinitions: Record<string, BrandDefinition> = {
  "claude-code": {
    iconSrc: claudeCodeIcon,
    accentClassName: "bg-card text-foreground",
  },
  codex: {
    iconSrc: codexIcon,
    accentClassName: "bg-card text-foreground",
  },
  "kimi-cli": {
    iconSrc: kimiIcon,
    accentClassName: "bg-neutral-950 text-white",
  },
  openclaw: {
    iconSrc: openClawIcon,
    accentClassName: "bg-card text-foreground",
  },
  opencode: {
    iconSrc: openCodeIcon,
    accentClassName: "bg-card text-foreground",
  },
};

interface BrandMarqueeProps {
  ariaLabel: string;
  items: IntegrationItem[];
}

function BrandLogo({ item }: { item: IntegrationItem }) {
  const definition = brandDefinitions[item.id];
  const logoSrc = definition?.iconSrc;

  return (
    <article
      className={cn(
        "flex min-w-[18rem] items-center gap-4 rounded-[1.75rem] border border-border/80 bg-surface px-5 py-4 text-foreground shadow-[0_10px_24px_rgba(17,19,21,0.05)]",
      )}
    >
      <div
        className={cn(
          "flex size-12 shrink-0 items-center justify-center rounded-2xl border border-white/70 bg-white/90 shadow-sm",
          definition?.accentClassName,
        )}
      >
        {logoSrc ? (
          <img alt="" className="size-7" loading="lazy" src={logoSrc} />
        ) : (
          <span className="text-sm font-semibold uppercase tracking-[0.16em]">{item.label.slice(0, 2)}</span>
        )}
      </div>

      <div className="flex min-w-0 flex-col gap-1">
        <p className="truncate text-[0.875rem] font-semibold tracking-[0.06em] uppercase">{item.label}</p>
        <p className="text-[0.8125rem] leading-5 text-current/70">{item.description}</p>
      </div>
    </article>
  );
}

export function BrandMarquee({ ariaLabel, items }: BrandMarqueeProps) {
  return (
    <div aria-label={ariaLabel} className="relative mt-12" role="group">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-y-0 left-0 z-10 hidden w-24 bg-[linear-gradient(90deg,#f4f3ec,transparent)] lg:block"
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-y-0 right-0 z-10 hidden w-24 bg-[linear-gradient(270deg,#f4f3ec,transparent)] lg:block"
      />

      <div className="hidden flex-wrap items-stretch justify-center gap-4 px-6 motion-reduce:flex">
        {items.map((item) => (
          <BrandLogo key={item.id} item={item} />
        ))}
      </div>

      <div className="motion-reduce:hidden">
        <Marquee className="brand-marquee-mask [--duration:34s]" pauseOnHover repeat={2}>
          {items.map((item) => (
            <BrandLogo key={item.id} item={item} />
          ))}
        </Marquee>
      </div>
    </div>
  );
}
