import {
  encodeDynamicModelId,
  MODELS,
  type ModelInfo,
  type ProviderId,
} from "../config";
import { createProxyFetch } from "./proxyFetch";

export type DiscoverModelsOptions = {
  apiKey?: string | null;
  baseURL?: string;
};

type RawModel = {
  id: string;
  name?: string;
  context?: number;
  reasoning?: boolean;
  tools?: boolean;
  vision?: boolean;
};

const OPENAI_COMPAT_BASE: Partial<Record<ProviderId, string>> = {
  openai: "https://api.openai.com/v1",
  xai: "https://api.x.ai/v1",
  cerebras: "https://api.cerebras.ai/v1",
  groq: "https://api.groq.com/openai/v1",
  deepseek: "https://api.deepseek.com",
  mistral: "https://api.mistral.ai/v1",
  openrouter: "https://openrouter.ai/api/v1",
  "opencode-zen": "https://opencode.ai/zen/v1",
  "llama-cpp": "http://127.0.0.1:8080/v1",
  lmstudio: "http://localhost:1234/v1",
  mlx: "http://127.0.0.1:8080/v1",
  ollama: "http://localhost:11434/v1",
};

const localProxyFetch = createProxyFetch({ allowPrivateNetwork: true });

export async function discoverProviderModels(
  provider: ProviderId,
  options: DiscoverModelsOptions = {},
): Promise<ModelInfo[]> {
  const raw = await fetchRawModels(provider, options);
  return mergeWithCurated(provider, raw);
}

function mergeWithCurated(provider: ProviderId, rawModels: RawModel[]): ModelInfo[] {
  const curated: ModelInfo[] = MODELS.filter((m) => m.provider === provider);
  const out: ModelInfo[] = [...curated];
  const seen = new Set(curated.map((m) => m.id));
  for (const raw of rawModels) {
    if (!raw.id || seen.has(raw.id)) continue;
    seen.add(raw.id);
    out.push({
      id: encodeDynamicModelId(provider, raw.id),
      provider,
      label: raw.name ?? humanizeModelId(raw.id),
      hint: "Live",
      description: raw.context
        ? `Discovered from provider API, ${formatContext(raw.context)} context.`
        : "Discovered from provider API.",
      capabilities: { intelligence: 3, speed: 3, cost: 3 },
      tags: [
        ...(raw.vision ? ["vision" as const] : []),
        ...(raw.reasoning ? ["reasoning" as const] : []),
        ...(raw.tools ? ["tools" as const] : []),
      ],
    });
  }
  return out;
}

async function fetchRawModels(
  provider: ProviderId,
  options: DiscoverModelsOptions,
): Promise<RawModel[]> {
  if (provider === "google") return fetchGoogleModels(options.apiKey);
  if (provider === "anthropic") return fetchAnthropicModels(options.apiKey);
  if (provider === "opencode-zen") return fetchOpenCodeModels(options.apiKey);
  if (provider === "openai-compatible") {
    if (!options.baseURL?.trim()) return [];
    return fetchOpenAICompatibleModels(provider, options.baseURL, options.apiKey);
  }
  const baseURL = options.baseURL?.trim() || OPENAI_COMPAT_BASE[provider];
  if (!baseURL) return [];
  return fetchOpenAICompatibleModels(provider, baseURL, options.apiKey);
}

async function fetchOpenAICompatibleModels(
  provider: ProviderId,
  baseURL: string,
  apiKey?: string | null,
): Promise<RawModel[]> {
  const headers: Record<string, string> = {};
  if (apiKey) headers.Authorization = `Bearer ${apiKey}`;
  const res = await localProxyFetch(`${baseURL.replace(/\/+$/, "")}/models`, {
    headers,
  });
  if (!res.ok) throw new Error(`${provider}: model list failed (${res.status})`);
  const json = await res.json();
  const data = Array.isArray(json?.data)
    ? json.data
    : Array.isArray(json?.models)
      ? json.models
      : [];
  return data
    .map(
      (m: {
        id?: unknown;
        name?: unknown;
        context_length?: unknown;
        max_context_length?: unknown;
      }) => ({
        id: typeof m.id === "string" ? m.id : "",
        name: typeof m.name === "string" ? m.name : undefined,
        context:
          typeof m.context_length === "number"
            ? m.context_length
            : typeof m.max_context_length === "number"
              ? m.max_context_length
              : undefined,
      }),
    )
    .filter((m: RawModel) => m.id);
}

async function fetchGoogleModels(apiKey?: string | null): Promise<RawModel[]> {
  if (!apiKey) return [];
  const res = await fetch(
    `https://generativelanguage.googleapis.com/v1beta/models?key=${encodeURIComponent(apiKey)}`,
  );
  if (!res.ok) throw new Error(`google: model list failed (${res.status})`);
  const json = await res.json();
  const models = Array.isArray(json?.models) ? json.models : [];
  return models
    .map((m: { name?: unknown; displayName?: unknown; supportedGenerationMethods?: unknown }) => {
      const name = typeof m.name === "string" ? m.name.replace(/^models\//, "") : "";
      const methods = Array.isArray(m.supportedGenerationMethods)
        ? m.supportedGenerationMethods
        : [];
      return {
        id: name,
        name: typeof m.displayName === "string" ? m.displayName : undefined,
        tools: methods.includes("generateContent"),
        vision: /gemini|gemma/i.test(name),
      };
    })
    .filter((m: RawModel) => m.id);
}

async function fetchAnthropicModels(apiKey?: string | null): Promise<RawModel[]> {
  if (!apiKey) return [];
  const res = await fetch("https://api.anthropic.com/v1/models", {
    headers: {
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
    },
  });
  if (!res.ok) throw new Error(`anthropic: model list failed (${res.status})`);
  const json = await res.json();
  const data = Array.isArray(json?.data) ? json.data : [];
  return data
    .map((m: { id?: unknown; display_name?: unknown }) => ({
      id: typeof m.id === "string" ? m.id : "",
      name: typeof m.display_name === "string" ? m.display_name : undefined,
      tools: true,
    }))
    .filter((m: RawModel) => m.id);
}

async function fetchOpenCodeModels(apiKey?: string | null): Promise<RawModel[]> {
  try {
    return await fetchOpenAICompatibleModels(
      "opencode-zen",
      OPENAI_COMPAT_BASE["opencode-zen"]!,
      apiKey,
    );
  } catch {
    const res = await fetch("https://models.dev/api.json");
    if (!res.ok) throw new Error(`opencode-zen: model list failed (${res.status})`);
    const json = await res.json();
    const models = json?.opencode?.models;
    if (!models || typeof models !== "object") return [];
    return Object.values(models).map((m) => {
      const item = m as {
        id?: unknown;
        name?: unknown;
        reasoning?: unknown;
        tool_call?: unknown;
        modalities?: { input?: unknown };
        limit?: { context?: unknown };
      };
      const input = Array.isArray(item.modalities?.input)
        ? item.modalities.input
        : [];
      return {
        id: typeof item.id === "string" ? item.id : "",
        name: typeof item.name === "string" ? item.name : undefined,
        context:
          typeof item.limit?.context === "number" ? item.limit.context : undefined,
        reasoning: item.reasoning === true,
        tools: item.tool_call === true,
        vision: input.includes("image"),
      };
    }).filter((m) => m.id);
  }
}

function humanizeModelId(id: string): string {
  return id
    .split(/[\/:_-]+/)
    .filter(Boolean)
    .map((part) => part.toUpperCase() === part ? part : part[0]?.toUpperCase() + part.slice(1))
    .join(" ");
}

function formatContext(tokens: number): string {
  return tokens >= 1_000_000
    ? `${Math.round(tokens / 1_000_000)}M`
    : `${Math.round(tokens / 1000)}K`;
}
