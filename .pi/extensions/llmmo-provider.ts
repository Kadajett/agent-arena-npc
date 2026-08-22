import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  pi.registerProvider("llmmo", {
    name: "LLMMO local Qwen",
    baseUrl: "https://llmmo.thehotpools.com/v1",
    apiKey: "$LLMMO_API_KEY",
    authHeader: true,
    api: "openai-completions",
    headers: {
      "CF-Access-Client-Id": "$LLMMO_CF_ACCESS_CLIENT_ID",
      "CF-Access-Client-Secret": "$LLMMO_CF_ACCESS_CLIENT_SECRET",
    },
    models: [
      {
        id: "qwen3.8-27b",
        name: "Qwen3.8 27B (house)",
        contextWindow: 12800,
        maxTokens: 3200,
        reasoning: true,
        input: ["text"],
        cost: { input: 0, output: 0 },
      },
    ],
  });
}
