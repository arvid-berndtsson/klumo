import { readFile } from "node:fs/promises";
import process from "node:process";

async function readStdin() {
  if (process.stdin.isTTY) {
    return null;
  }

  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString("utf8");
}

function inferLanguageFromPath(filePath) {
  const idx = filePath.lastIndexOf(".");
  if (idx === -1) return null;
  const ext = filePath.slice(idx + 1).toLowerCase();
  if (!ext) return null;
  return ext;
}

export async function readFromArgvOrStdin(filePath) {
  if (filePath) {
    const sourceText = await readFile(filePath, "utf8");
    return {
      sourceText,
      sourceId: filePath,
      inferredLang: inferLanguageFromPath(filePath)
    };
  }

  const stdinText = await readStdin();
  if (stdinText === null || stdinText.trim().length === 0) {
    throw new Error("No input provided. Pass a file path or pipe input via stdin.");
  }

  return {
    sourceText: stdinText,
    sourceId: "stdin",
    inferredLang: null
  };
}
