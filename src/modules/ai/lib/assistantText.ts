const THINK_BLOCK_TAGS = "think|thinking|reasoning|reflection";

const THINK_BLOCK_CAPTURE_RE = new RegExp(
  `<(${THINK_BLOCK_TAGS})\\b[^>]*>([\\s\\S]*?)<\\/\\1>`,
  "gi",
);
const OPEN_THINK_BLOCK_RE = new RegExp(
  `<(${THINK_BLOCK_TAGS})\\b[^>]*>([\\s\\S]*)$`,
  "i",
);

const BRACKET_THINK_BLOCK_CAPTURE_RE =
  /\[(think|thinking|reasoning|reflection)\]([\s\S]*?)\[\/\1\]/gi;
const OPEN_BRACKET_THINK_BLOCK_RE =
  /\[(think|thinking|reasoning|reflection)\]([\s\S]*)$/i;

function normalizeText(text: string): string {
  return text.replace(/\n{3,}/g, "\n\n").trim();
}

export function splitAssistantThinking(text: string): {
  reasoning: string;
  answer: string;
} {
  if (!text) return { reasoning: "", answer: "" };

  const reasoningParts: string[] = [];

  let answer = text.replace(
    THINK_BLOCK_CAPTURE_RE,
    (_all: string, _tag: string, inner: string) => {
      const v = normalizeText(inner ?? "");
      if (v) reasoningParts.push(v);
      return "";
    },
  );

  answer = answer.replace(
    BRACKET_THINK_BLOCK_CAPTURE_RE,
    (_all: string, _tag: string, inner: string) => {
      const v = normalizeText(inner ?? "");
      if (v) reasoningParts.push(v);
      return "";
    },
  );

  // Handle incomplete streaming fragments like: "<think>..."
  const openTagMatch = answer.match(OPEN_THINK_BLOCK_RE);
  if (openTagMatch) {
    const v = normalizeText(openTagMatch[2] ?? "");
    if (v) reasoningParts.push(v);
    answer = answer.replace(OPEN_THINK_BLOCK_RE, "");
  }

  const openBracketMatch = answer.match(OPEN_BRACKET_THINK_BLOCK_RE);
  if (openBracketMatch) {
    const v = normalizeText(openBracketMatch[2] ?? "");
    if (v) reasoningParts.push(v);
    answer = answer.replace(OPEN_BRACKET_THINK_BLOCK_RE, "");
  }

  return {
    reasoning: normalizeText(reasoningParts.join("\n\n")),
    answer: normalizeText(answer),
  };
}

export function stripAssistantThinking(text: string): string {
  return splitAssistantThinking(text).answer;
}
