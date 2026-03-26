export const supportedLocales = ["en", "zh", "fr"] as const;

export type Locale = (typeof supportedLocales)[number];

export const docsUrl = "https://docs.chainbot.dev";
export const githubUrl = "https://github.com/chainbotlab/chainbot";
export const homepageUrl = "https://chainbot.dev";

export const localeHref: Record<Locale, string> = {
  en: "/",
  zh: "/zh/",
  fr: "/fr/",
};

export const localeLabels: Record<Locale, string> = {
  en: "EN",
  zh: "中文",
  fr: "FR",
};

export type InstallStep = {
  command: string;
  description: string;
};

export type ArchitectureStepTone = "base" | "accent" | "ink";

export type ArchitectureStep = {
  id: string;
  title: string;
  description: string;
  tone: ArchitectureStepTone;
};

export type IntegrationItem = {
  id: string;
  label: string;
  description: string;
};

export type SiteContent = {
  meta: {
    title: string;
    description: string;
  };
  brand: {
    name: string;
    eyebrow: string;
    footerName: string;
    footerCopyright: string;
  };
  navigation: {
    docsLabel: string;
    githubLabel: string;
    languageLabel: string;
    docsDomain: string;
  };
  hero: {
    badge: string;
    title: string;
    description: string;
    chips: string[];
  };
  install: {
    eyebrow: string;
    title: string;
    description: string;
    primaryCommand: string;
    nextStepsLabel: string;
    nextSteps: InstallStep[];
    docsLabel: string;
    copyLabel: string;
    copiedLabel: string;
    copyFailedLabel: string;
  };
  architecture: {
    eyebrow: string;
    title: string;
    description: string;
    tags: string[];
    steps: ArchitectureStep[];
  };
  integrations: {
    items: IntegrationItem[];
  };
  cta: {
    title: string;
    description: string;
    command: string;
    docsLabel: string;
  };
  footer: {
    summary: string;
    docsLabel: string;
    githubLabel: string;
  };
};

const integrations: IntegrationItem[] = [
  { id: "claude-code", label: "Claude Code", description: "Anthropic coding agent" },
  { id: "codex", label: "Codex", description: "OpenAI engineering workflow" },
  { id: "kimi-cli", label: "Kimi CLI", description: "Moonshot terminal copilot" },
  { id: "openclaw", label: "OpenClaw", description: "Autonomous terminal assistant" },
  { id: "opencode", label: "OpenCode", description: "Open agent runtime surface" },
];

const siteContent: Record<Locale, SiteContent> = {
  en: {
    meta: {
      title: "ChainBot CLI — command line orchestration for operator workflows",
      description:
        "ChainBot is an open-source CLI for orchestrating triggers, workflows, and plugins without hiding runtime logic in fragile shell scripts.",
    },
    brand: {
      name: "ChainBot CLI",
      eyebrow: "Developer Tooling",
      footerName: "ChainBot",
      footerCopyright: "© 2026 ChainBot",
    },
    navigation: {
      docsLabel: "Docs",
      githubLabel: "GitHub",
      languageLabel: "Language",
      docsDomain: "docs.chainbot.dev",
    },
    hero: {
      badge: "Next-Gen Terminal Utility",
      title: "Run triggers, workflows, and custom plugins from one reliable CLI.",
      description:
        "ChainBot is an open-source CLI that turns shell-script glue into explicit workflow execution with durable runtime state, trigger control, and operator-readable history.",
      chips: ["Trigger-driven runs", "Composable workflows", "Extensible plugins"],
    },
    install: {
      eyebrow: "Quick Install",
      title: "Install ChainBot CLI",
      description: "Get started from source or Cargo and bootstrap a workspace in seconds.",
      primaryCommand: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      nextStepsLabel: "Next Steps",
      nextSteps: [
        { command: "chainbot init", description: "Initialize workspace" },
        { command: "chainbot run", description: "Execute workflow" },
        { command: "chainbot observe", description: "Inspect runtime history" },
      ],
      docsLabel: "Read the full documentation",
      copyLabel: "Copy to clipboard",
      copiedLabel: "Copied",
      copyFailedLabel: "Copy failed",
    },
    architecture: {
      eyebrow: "Architecture",
      title: "Declarative pipelines, readable execution",
      description:
        "ChainBot keeps triggers, workflows, plugin operations, and runtime history aligned in one durable control surface. No more digging through ad-hoc scripts to understand what failed.",
      tags: ["Trigger state", "Workflow graphs", "Plugin runtime", "Audit trails"],
      steps: [
        {
          id: "01",
          title: "Trigger",
          description:
            "Cron schedules, webhooks, or manual operator input start the run from a stable ChainBot entrypoint.",
          tone: "base",
        },
        {
          id: "02",
          title: "Workflow Graph",
          description:
            "Compose repeatable steps as explicit workflow nodes instead of hidden, nested scripts.",
          tone: "accent",
        },
        {
          id: "03",
          title: "Reliable Run",
          description:
            "Keep trigger state, validation, and execution flow visible while plugins and operators collaborate safely.",
          tone: "ink",
        },
      ],
    },
    integrations: {
      items: integrations,
    },
    cta: {
      title: "Ready to simplify your pipelines?",
      description:
        "Use ChainBot to manage complex operator workflows, trigger execution, and custom plugin pipelines from one durable CLI surface.",
      command: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      docsLabel: "Read Documentation",
    },
    footer: {
      summary:
        "ChainBot is an open-source CLI for teams coordinating triggers, workflows, and plugin-driven automation without hiding logic in scripts.",
      docsLabel: "Documentation",
      githubLabel: "GitHub",
    },
  },
  zh: {
    meta: {
      title: "ChainBot CLI — 面向运维工作流编排的命令行工具",
      description:
        "ChainBot 是一个开源 CLI，用显式 trigger、workflow 与 plugin 执行替代脆弱的 shell 脚本。",
    },
    brand: {
      name: "ChainBot CLI",
      eyebrow: "Developer Tooling",
      footerName: "ChainBot",
      footerCopyright: "© 2026 ChainBot",
    },
    navigation: {
      docsLabel: "文档",
      githubLabel: "GitHub",
      languageLabel: "语言",
      docsDomain: "docs.chainbot.dev",
    },
    hero: {
      badge: "新一代终端工具",
      title: "在一个可靠 CLI 中运行 trigger、workflow 与自定义 plugin。",
      description:
        "ChainBot 是一个开源 CLI，把易碎脚本整理成显式工作流执行，并持续保留运行状态、触发器控制与可读历史。",
      chips: ["触发驱动运行", "可组合工作流", "可扩展插件"],
    },
    install: {
      eyebrow: "快速安装",
      title: "安装 ChainBot CLI",
      description: "通过本地源码或 Cargo 快速安装，并立即初始化 workspace。",
      primaryCommand: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      nextStepsLabel: "下一步",
      nextSteps: [
        { command: "chainbot init", description: "初始化工作空间" },
        { command: "chainbot run", description: "执行工作流" },
        { command: "chainbot observe", description: "查看运行历史" },
      ],
      docsLabel: "阅读完整文档",
      copyLabel: "复制命令",
      copiedLabel: "已复制",
      copyFailedLabel: "复制失败",
    },
    architecture: {
      eyebrow: "架构",
      title: "声明式流水线，可读的执行过程",
      description:
        "ChainBot 把 trigger、workflow、plugin operation 与运行历史对齐在同一个持久控制面里，不再需要翻遍脚本才能知道哪里出错。",
      tags: ["触发器状态", "工作流图", "插件运行时", "审计轨迹"],
      steps: [
        {
          id: "01",
          title: "Trigger",
          description: "定时任务、Webhook 或手动操作都能从稳定的 ChainBot 入口发起一次运行。",
          tone: "base",
        },
        {
          id: "02",
          title: "Workflow Graph",
          description: "把可重复步骤组织成显式 workflow 节点，而不是隐藏的嵌套脚本。",
          tone: "accent",
        },
        {
          id: "03",
          title: "Reliable Run",
          description: "在 plugin 和操作员协同时，持续保持状态、校验与执行流清晰可见。",
          tone: "ink",
        },
      ],
    },
    integrations: {
      items: integrations,
    },
    cta: {
      title: "准备好简化你的流水线了吗？",
      description: "用 ChainBot 在一个统一 CLI 中管理复杂工作流、触发器执行和自定义插件流水线。",
      command: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      docsLabel: "查看文档",
    },
    footer: {
      summary: "ChainBot 是一个开源 CLI，帮助团队管理 trigger、workflow 与插件自动化，而不必把关键逻辑藏进脚本里。",
      docsLabel: "文档",
      githubLabel: "GitHub",
    },
  },
  fr: {
    meta: {
      title: "ChainBot CLI — orchestration en ligne de commande pour workflows opérateurs",
      description:
        "ChainBot est un CLI open source qui remplace les scripts shell fragiles par des triggers, workflows et plugins explicites.",
    },
    brand: {
      name: "ChainBot CLI",
      eyebrow: "Developer Tooling",
      footerName: "ChainBot",
      footerCopyright: "© 2026 ChainBot",
    },
    navigation: {
      docsLabel: "Docs",
      githubLabel: "GitHub",
      languageLabel: "Langue",
      docsDomain: "docs.chainbot.dev",
    },
    hero: {
      badge: "Utilitaire terminal nouvelle génération",
      title: "Exécutez triggers, workflows et plugins personnalisés depuis un CLI fiable.",
      description:
        "ChainBot est un CLI open source qui transforme la glue shell fragile en exécution explicite avec état durable, contrôle des triggers et historique lisible.",
      chips: ["Runs pilotés par trigger", "Workflows composables", "Plugins extensibles"],
    },
    install: {
      eyebrow: "Installation rapide",
      title: "Installer ChainBot CLI",
      description: "Installez depuis la source ou via Cargo et initialisez un workspace en quelques secondes.",
      primaryCommand: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      nextStepsLabel: "Étapes suivantes",
      nextSteps: [
        { command: "chainbot init", description: "Initialiser le workspace" },
        { command: "chainbot run", description: "Exécuter le workflow" },
        { command: "chainbot observe", description: "Inspecter l'historique" },
      ],
      docsLabel: "Lire la documentation complète",
      copyLabel: "Copier la commande",
      copiedLabel: "Copié",
      copyFailedLabel: "Échec de la copie",
    },
    architecture: {
      eyebrow: "Architecture",
      title: "Pipelines déclaratifs, exécution lisible",
      description:
        "ChainBot garde triggers, workflows, opérations plugin et historique d'exécution alignés dans une surface de contrôle durable. Plus besoin de fouiller des scripts pour comprendre un échec.",
      tags: ["État des triggers", "Graphes de workflow", "Runtime plugin", "Pistes d'audit"],
      steps: [
        {
          id: "01",
          title: "Trigger",
          description:
            "Cron, webhooks ou action manuelle démarrent chaque exécution depuis un point d'entrée ChainBot stable.",
          tone: "base",
        },
        {
          id: "02",
          title: "Workflow Graph",
          description:
            "Composez des étapes répétables sous forme de nœuds de workflow explicites au lieu de scripts imbriqués cachés.",
          tone: "accent",
        },
        {
          id: "03",
          title: "Reliable Run",
          description:
            "Gardez l'état, la validation et le flux d'exécution visibles pendant que plugins et opérateurs collaborent en sécurité.",
          tone: "ink",
        },
      ],
    },
    integrations: {
      items: integrations,
    },
    cta: {
      title: "Prêt à simplifier vos pipelines ?",
      description:
        "Utilisez ChainBot pour gérer workflows opérateurs, exécutions déclenchées et pipelines de plugins personnalisés dans une seule surface CLI durable.",
      command: "cargo install --git https://github.com/chainbotlab/chainbot chainbot",
      docsLabel: "Lire la documentation",
    },
    footer: {
      summary:
        "ChainBot est un CLI open source pour les équipes qui coordonnent triggers, workflows et automatisation par plugins sans cacher la logique dans des scripts.",
      docsLabel: "Documentation",
      githubLabel: "GitHub",
    },
  },
};

export function getSiteContent(locale: Locale): SiteContent {
  return siteContent[locale];
}
