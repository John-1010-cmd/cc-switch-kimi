import React from "react";
import type { AppId } from "@/lib/api/types";
import { ClaudeIcon, CodexIcon } from "@/components/BrandIcons";
import { ProviderIcon } from "@/components/ProviderIcon";

export interface AppConfig {
  label: string;
  icon: React.ReactNode;
  activeClass: string;
  badgeClass: string;
}

export const APP_IDS: AppId[] = ["claude", "codex", "kimi-code"];

/** App IDs shown in Skills panels */
export const SKILLS_APP_IDS: AppId[] = ["claude", "codex"];

/** App IDs shown in MCP panels (Kimi Code 支持 MCP，随 M1 纳入) */
export const MCP_APP_IDS: AppId[] = [...SKILLS_APP_IDS, "kimi-code"];

export const APP_ICON_MAP: Record<AppId, AppConfig> = {
  claude: {
    label: "Claude",
    icon: <ClaudeIcon size={14} />,
    activeClass:
      "bg-orange-500/10 ring-1 ring-orange-500/20 hover:bg-orange-500/20 text-orange-600 dark:text-orange-400",
    badgeClass:
      "bg-orange-500/10 text-orange-700 dark:text-orange-300 hover:bg-orange-500/20 border-0 gap-1.5",
  },
  codex: {
    label: "Codex",
    icon: <CodexIcon size={14} />,
    activeClass:
      "bg-green-500/10 ring-1 ring-green-500/20 hover:bg-green-500/20 text-green-600 dark:text-green-400",
    badgeClass:
      "bg-green-500/10 text-green-700 dark:text-green-300 hover:bg-green-500/20 border-0 gap-1.5",
  },
  "kimi-code": {
    label: "Kimi Code",
    icon: (
      <ProviderIcon
        icon="kimi"
        name="Kimi Code"
        size={14}
        showFallback={false}
      />
    ),
    activeClass:
      "bg-violet-500/10 ring-1 ring-violet-500/20 hover:bg-violet-500/20 text-violet-600 dark:text-violet-400",
    badgeClass:
      "bg-violet-500/10 text-violet-700 dark:text-violet-300 hover:bg-violet-500/20 border-0 gap-1.5",
  },
};
