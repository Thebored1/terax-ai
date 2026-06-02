import type { ProviderId } from "@/modules/ai/config";
import {
  AppleIcon,
  ChatGptIcon,
  ClaudeIcon,
  ComputerIcon,
  FlashIcon,
  GoogleGeminiIcon,
  Grok02Icon,
  CpuIcon,
  DeepseekIcon,
  GlobeIcon,
  CodeIcon,
  MistralIcon,
  PlugIcon,
  ServerStack01Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";

const ICON_BY_PROVIDER = {
  openai: ChatGptIcon,
  anthropic: ClaudeIcon,
  google: GoogleGeminiIcon,
  xai: Grok02Icon,
  cerebras: CpuIcon,
  groq: FlashIcon,
  deepseek: DeepseekIcon,
  mistral: MistralIcon,
  "opencode-zen": CodeIcon,
  openrouter: GlobeIcon,
  "openai-compatible": PlugIcon,
  "llama-cpp": ServerStack01Icon,
  lmstudio: ComputerIcon,
  mlx: AppleIcon,
  ollama: ServerStack01Icon,
} as const satisfies Record<ProviderId, typeof ChatGptIcon>;

type Props = {
  provider: ProviderId;
  size?: number;
  className?: string;
};

export function ProviderIcon({ provider, size = 14, className }: Props) {
  return (
    <HugeiconsIcon
      icon={ICON_BY_PROVIDER[provider]}
      size={size}
      strokeWidth={1.75}
      className={className}
    />
  );
}
