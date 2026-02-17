import { resolve } from "node:path";
import { readFromArgvOrStdin } from "./input.mjs";
import { translateToJs } from "./translator.mjs";
import { getCachedTranslation, putCachedTranslation } from "./cache.mjs";
import { executeJavaScript } from "./executor.mjs";

function parseArgs(argv) {
  const args = {
    filePath: null,
    languageHint: null,
    noCache: false,
    printJs: false,
    help: false
  };

  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token === "--help" || token === "-h") {
      args.help = true;
    } else if (token === "--no-cache") {
      args.noCache = true;
    } else if (token === "--print-js") {
      args.printJs = true;
    } else if (token === "--lang" || token === "-l") {
      if (!argv[i + 1] || argv[i + 1].startsWith("-")) {
        throw new Error("--lang requires a value.");
      }
      args.languageHint = argv[i + 1] ?? null;
      i += 1;
    } else if (!args.filePath) {
      args.filePath = token;
    }
  }

  return args;
}

function printHelp() {
  console.log(`smnode - LLM-powered source-to-JavaScript runtime

Usage:
  smnode <file> [--lang <hint>] [--print-js] [--no-cache]
  cat source.any | smnode --lang pseudocode

Options:
  --lang, -l    Hint for source language (e.g. "python", "dsl", "pseudo")
  --print-js    Print generated JavaScript before executing
  --no-cache    Disable translation cache
  --help, -h    Show this help message

Environment:
  OPENAI_API_KEY        Required
  OPENAI_BASE_URL       Optional (default: https://api.openai.com/v1)
  SMART_NODE_MODEL      Optional (default: gpt-4.1-mini)
`);
}

export async function runCli(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    printHelp();
    return;
  }

  const { sourceText, sourceId, inferredLang } = await readFromArgvOrStdin(args.filePath);
  const languageHint = args.languageHint ?? inferredLang;
  const effectiveSourceId = args.filePath ? resolve(args.filePath) : sourceId;
  const cacheEnabled = !args.noCache;

  let generatedJs = null;
  if (cacheEnabled) {
    generatedJs = await getCachedTranslation(sourceText, languageHint, effectiveSourceId);
  }

  if (!generatedJs) {
    generatedJs = await translateToJs({
      sourceText,
      languageHint,
      sourceId: effectiveSourceId
    });

    if (cacheEnabled) {
      await putCachedTranslation(sourceText, languageHint, effectiveSourceId, generatedJs);
    }
  }

  if (args.printJs) {
    console.log("/* ===== generated JavaScript ===== */");
    console.log(generatedJs);
    console.log("/* ===== end generated JavaScript ===== */");
  }

  await executeJavaScript(generatedJs, {
    sourceId: effectiveSourceId
  });
}
