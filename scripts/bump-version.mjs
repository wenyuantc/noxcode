import fs from "node:fs";
import path from "node:path";
import readline from "node:readline/promises";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const updatedFiles = [];
const currentVersion = readCurrentVersion();
const nextVersion = await resolveNextVersion();

if (!isValidVersion(nextVersion)) {
  console.error(`无效版本号: ${nextVersion}`);
  process.exit(1);
}

function replaceInFile(relativePath, replacer) {
  const filePath = path.join(rootDir, relativePath);
  const original = fs.readFileSync(filePath, "utf8");
  const updated = replacer(original);

  if (updated === original) {
    return;
  }

  fs.writeFileSync(filePath, updated);
  updatedFiles.push(relativePath);
}

replaceInFile("src-tauri/Cargo.toml", (content) =>
  replaceAfterMarker(
    content,
    "[package]",
    /^version\s*=\s*"[^"]+"$/m,
    `version = "${nextVersion}"`,
  ),
);

replaceInFile("src-tauri/Cargo.lock", (content) =>
  replaceAfterMarker(
    content,
    '[[package]]\nname = "noxcode"',
    /^version = "[^"]+"$/m,
    `version = "${nextVersion}"`,
  ),
);

replaceInFile("package.json", (content) =>
  replaceLine(content, /^  "version": "[^"]+",$/m, `  "version": "${nextVersion}",`),
);

replaceInFile("package-lock.json", (content) =>
  replaceAfterMarker(
    replaceLine(content, /^  "version": "[^"]+",$/m, `  "version": "${nextVersion}",`),
    '  "": {',
    /^      "version": "[^"]+",$/m,
    `      "version": "${nextVersion}",`,
  ),
);

replaceInFile("src-tauri/tauri.conf.json", (content) =>
  replaceLine(content, /^  "version": "[^"]+",$/m, `  "version": "${nextVersion}",`),
);

if (updatedFiles.length === 0) {
  console.log(`版本号已经是 ${nextVersion}，无需更新`);
  process.exit(0);
}

console.log(`版本号已更新为 ${nextVersion}`);
for (const file of updatedFiles) {
  console.log(`- ${file}`);
}

function replaceAfterMarker(content, marker, pattern, replacement) {
  const markerIndex = content.indexOf(marker);

  if (markerIndex === -1) {
    return content;
  }

  const beforeMarker = content.slice(0, markerIndex);
  const afterMarker = content.slice(markerIndex);
  const match = afterMarker.match(pattern);

  if (!match || match[0] === replacement) {
    return content;
  }

  const updatedAfterMarker = afterMarker.replace(pattern, replacement);
  return beforeMarker + updatedAfterMarker;
}

function replaceLine(content, pattern, replacement) {
  const match = content.match(pattern);

  if (!match || match[0] === replacement) {
    return content;
  }

  return content.replace(pattern, replacement);
}

async function resolveNextVersion() {
  const cliVersion = process.argv[2]?.trim();

  if (cliVersion) {
    return cliVersion;
  }

  if (!process.stdin.isTTY || !process.stdout.isTTY) {
    console.error("用法: npm run bump-version -- <version>");
    process.exit(1);
  }

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  try {
    const answer = await rl.question(`请输入新版本号（当前 ${currentVersion}）: `);
    const version = answer.trim();

    if (!version) {
      console.error("未输入版本号，已取消");
      process.exit(1);
    }

    return version;
  } finally {
    rl.close();
  }
}

function readCurrentVersion() {
  const packageJsonPath = path.join(rootDir, "package.json");
  const content = fs.readFileSync(packageJsonPath, "utf8");
  const match = content.match(/^  "version": "([^"]+)",$/m);

  return match?.[1] ?? "未知";
}

function isValidVersion(version) {
  return /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version);
}
