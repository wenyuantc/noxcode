import fs from "node:fs";
import path from "node:path";

const args = parseArgs(process.argv.slice(2));
const inputDir = requiredPath(
  args.input ?? process.env.LATEST_JSON_INPUT,
  "--input or LATEST_JSON_INPUT",
);
const outputPath = args.output ?? process.env.LATEST_JSON_OUTPUT ?? "latest.json";
const versionInput =
  args.version ?? process.env.LATEST_JSON_VERSION ?? process.env.GITHUB_REF_NAME ?? "";
const version = stripLeadingV(versionInput);
const notes =
  args.notes ?? process.env.LATEST_JSON_NOTES ?? (versionInput ? String(versionInput) : version);
const assetBaseUrl = resolveAssetBaseUrl(
  args["asset-base-url"] ?? process.env.LATEST_JSON_ASSET_BASE_URL,
  versionInput || version,
);

if (!version) {
  console.error("缺少版本号：请传入 --version、LATEST_JSON_VERSION 或 GITHUB_REF_NAME");
  process.exit(1);
}

if (!fs.existsSync(inputDir) || !fs.statSync(inputDir).isDirectory()) {
  console.error(`输入目录不存在: ${inputDir}`);
  process.exit(1);
}

const files = listFiles(inputDir).sort((left, right) => left.localeCompare(right));
const platforms = {};
const darwin = pickPlatform(files, (filePath) => hasSuffix(filePath, ".app.tar.gz"));
const linux = pickPlatform(files, (filePath) => hasSuffix(filePath, ".AppImage"));
const windows = pickPlatform(
  files,
  (filePath) => hasSuffix(filePath, ".nsis.zip"),
  (filePath) => hasSuffix(filePath, "-setup.exe") || hasSuffix(filePath, ".exe"),
);

if (darwin) {
  platforms["darwin-aarch64"] = darwin;
}
if (linux) {
  platforms["linux-x86_64"] = linux;
}
if (windows) {
  platforms["windows-x86_64"] = windows;
}

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms,
};

fs.mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Wrote ${outputPath}`);
console.log(`version=${version}`);
console.log(`platforms=${Object.keys(platforms).join(",") || "(none)"}`);

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      continue;
    }
    const key = token.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) {
      parsed[key] = "true";
      continue;
    }
    parsed[key] = next;
    index += 1;
  }
  return parsed;
}

function requiredPath(value, label) {
  if (!value) {
    console.error(`缺少输入目录：请传入 ${label}`);
    process.exit(1);
  }
  return value;
}

function stripLeadingV(value) {
  return String(value ?? "")
    .trim()
    .replace(/^v/i, "");
}

function resolveAssetBaseUrl(explicit, versionTag) {
  if (explicit) {
    return explicit.replace(/\/$/, "");
  }
  const repository = process.env.GITHUB_REPOSITORY;
  const tag = String(versionTag ?? "").trim();
  const releaseTag = tag ? (tag.startsWith("v") ? tag : `v${tag}`) : "";
  if (!repository || !releaseTag) {
    console.error(
      "缺少 --asset-base-url / LATEST_JSON_ASSET_BASE_URL（或 GITHUB_REPOSITORY + version）",
    );
    process.exit(1);
  }
  return `https://github.com/${repository}/releases/download/${releaseTag}`;
}

function listFiles(dir) {
  const collected = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      collected.push(...listFiles(fullPath));
    } else if (entry.isFile()) {
      collected.push(fullPath);
    }
  }
  return collected;
}

function hasSuffix(filePath, suffix) {
  return path.basename(filePath).toLowerCase().endsWith(suffix.toLowerCase());
}

function githubReleaseAssetName(filename) {
  // GitHub Release 会把上传资源名里的空格替换成 `.`，下载 URL 必须用替换后的名字。
  return filename.replaceAll(" ", ".");
}

function pickPlatform(files, ...matchers) {
  for (const matcher of matchers) {
    const artifact = files.find((filePath) => matcher(filePath) && !hasSuffix(filePath, ".sig"));
    if (!artifact) {
      continue;
    }
    const signaturePath = `${artifact}.sig`;
    if (!fs.existsSync(signaturePath)) {
      continue;
    }
    const signature = fs.readFileSync(signaturePath, "utf8").trim();
    if (!signature) {
      continue;
    }
    return {
      signature,
      url: `${assetBaseUrl}/${encodeURIComponent(githubReleaseAssetName(path.basename(artifact)))}`,
    };
  }
  return null;
}
