import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

const CACHE_DIR = join(homedir(), ".smart-node-cache");

function buildCacheKey(sourceText, languageHint, sourceId) {
  const hash = createHash("sha256");
  hash.update(sourceText);
  hash.update("\n--lang--\n");
  hash.update(languageHint ?? "unknown");
  hash.update("\n--source--\n");
  hash.update(sourceId);
  return hash.digest("hex");
}

function cachePathForKey(key) {
  return join(CACHE_DIR, `${key}.json`);
}

export async function getCachedTranslation(sourceText, languageHint, sourceId) {
  const key = buildCacheKey(sourceText, languageHint, sourceId);
  const path = cachePathForKey(key);

  try {
    const content = await readFile(path, "utf8");
    const parsed = JSON.parse(content);
    return typeof parsed.generatedJs === "string" ? parsed.generatedJs : null;
  } catch {
    return null;
  }
}

export async function putCachedTranslation(sourceText, languageHint, sourceId, generatedJs) {
  const key = buildCacheKey(sourceText, languageHint, sourceId);
  const path = cachePathForKey(key);
  await mkdir(CACHE_DIR, { recursive: true });
  await writeFile(
    path,
    JSON.stringify(
      {
        createdAt: new Date().toISOString(),
        sourceId,
        languageHint,
        generatedJs
      },
      null,
      2
    ),
    "utf8"
  );
}
