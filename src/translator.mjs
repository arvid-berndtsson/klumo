function extractCodeBlock(text) {
  const block = text.match(/```(?:javascript|js)?\s*([\s\S]*?)```/i);
  if (block && block[1]) {
    return block[1].trim();
  }
  return text.trim();
}

function buildPrompt({ sourceText, languageHint, sourceId }) {
  const langText = languageHint ? `Language hint: ${languageHint}` : "Language hint: unknown";

  return [
    "You are a strict transpiler.",
    "Task: convert input source into runnable modern JavaScript (Node.js ESM).",
    "Return only JavaScript code and no explanation.",
    "Preserve behavior as closely as possible.",
    "If the source is ambiguous, choose a practical interpretation.",
    "Do not include markdown fences unless unavoidable.",
    `Source id: ${sourceId}`,
    langText,
    "",
    "INPUT START",
    sourceText,
    "INPUT END"
  ].join("\n");
}

async function callOpenAI({ model, apiKey, baseUrl, prompt }) {
  const response = await fetch(`${baseUrl}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey}`
    },
    body: JSON.stringify({
      model,
      temperature: 0,
      messages: [
        {
          role: "system",
          content:
            "You convert arbitrary source text into executable Node.js JavaScript. Output code only."
        },
        {
          role: "user",
          content: prompt
        }
      ]
    })
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(`LLM request failed (${response.status}): ${body}`);
  }

  const json = await response.json();
  const content = json?.choices?.[0]?.message?.content;
  if (typeof content !== "string" || !content.trim()) {
    throw new Error("LLM returned empty response.");
  }
  return content;
}

export async function translateToJs({ sourceText, languageHint, sourceId }) {
  const apiKey = process.env.OPENAI_API_KEY;
  const baseUrl = process.env.OPENAI_BASE_URL ?? "https://api.openai.com/v1";
  const model = process.env.SMART_NODE_MODEL ?? "gpt-4.1-mini";

  if (!apiKey) {
    throw new Error("OPENAI_API_KEY is required.");
  }

  const prompt = buildPrompt({ sourceText, languageHint, sourceId });
  const raw = await callOpenAI({ model, apiKey, baseUrl, prompt });
  const generatedJs = extractCodeBlock(raw);

  if (!generatedJs) {
    throw new Error("Translation produced empty JavaScript.");
  }

  return generatedJs;
}
