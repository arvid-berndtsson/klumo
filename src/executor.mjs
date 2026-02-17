import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";

export async function executeJavaScript(generatedJs, { sourceId }) {
  const dir = await mkdtemp(join(tmpdir(), "smart-node-"));
  const filePath = join(dir, "generated.mjs");
  const wrapped = `// Generated from: ${sourceId}\n${generatedJs}\n`;
  await writeFile(filePath, wrapped, "utf8");

  try {
    await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [filePath], {
        stdio: "inherit"
      });

      child.on("error", reject);
      child.on("close", (code) => {
        if (code === 0) {
          resolve();
          return;
        }
        reject(new Error(`Execution failed with exit code ${code}.`));
      });
    });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
