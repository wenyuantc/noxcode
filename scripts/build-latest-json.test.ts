import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const scriptPath = path.resolve(fileURLToPath(new URL("./build-latest-json.mjs", import.meta.url)));

let tempDir: string | null = null;

afterEach(() => {
  if (tempDir) {
    fs.rmSync(tempDir, { recursive: true, force: true });
    tempDir = null;
  }
});

function makeInputDir(): string {
  tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "latest-json-"));
  return tempDir;
}

function writeArtifact(dir: string, name: string, content = "artifact\n"): void {
  fs.writeFileSync(path.join(dir, name), content);
}

function runScript(inputDir: string, outputPath: string, version = "v0.5.9"): void {
  execFileSync(
    process.execPath,
    [scriptPath, "--input", inputDir, "--output", outputPath, "--version", version],
    {
      env: { ...process.env, GITHUB_REPOSITORY: "wenyuantc/codex-ai" },
      encoding: "utf8",
    },
  );
}

describe("build-latest-json", () => {
  it("rewrites GitHub Release spaces to periods in download URLs", () => {
    const inputDir = makeInputDir();
    writeArtifact(inputDir, "Codex AI System.app.tar.gz");
    writeArtifact(inputDir, "Codex AI System.app.tar.gz.sig", "darwin-sig\n");
    writeArtifact(inputDir, "Codex AI System_0.5.9_amd64.AppImage");
    writeArtifact(inputDir, "Codex AI System_0.5.9_amd64.AppImage.sig", "linux-sig\n");
    writeArtifact(inputDir, "Codex AI System_0.5.9_x64-setup.exe");
    writeArtifact(inputDir, "Codex AI System_0.5.9_x64-setup.exe.sig", "windows-sig\n");

    const outputPath = path.join(inputDir, "latest.json");
    runScript(inputDir, outputPath);

    const manifest = JSON.parse(fs.readFileSync(outputPath, "utf8")) as {
      platforms: Record<string, { url: string }>;
    };
    const base = "https://github.com/wenyuantc/codex-ai/releases/download/v0.5.9";

    expect(manifest.platforms["darwin-aarch64"].url).toBe(`${base}/Codex.AI.System.app.tar.gz`);
    expect(manifest.platforms["linux-x86_64"].url).toBe(
      `${base}/Codex.AI.System_0.5.9_amd64.AppImage`,
    );
    expect(manifest.platforms["windows-x86_64"].url).toBe(
      `${base}/Codex.AI.System_0.5.9_x64-setup.exe`,
    );
    expect(fs.readFileSync(outputPath, "utf8")).not.toContain("%20");
  });

  it("keeps filenames without spaces unchanged", () => {
    const inputDir = makeInputDir();
    writeArtifact(inputDir, "Codex.AI.System.app.tar.gz");
    writeArtifact(inputDir, "Codex.AI.System.app.tar.gz.sig", "darwin-sig\n");

    const outputPath = path.join(inputDir, "latest.json");
    runScript(inputDir, outputPath);

    const manifest = JSON.parse(fs.readFileSync(outputPath, "utf8")) as {
      platforms: Record<string, { url: string }>;
    };
    expect(manifest.platforms["darwin-aarch64"].url).toBe(
      "https://github.com/wenyuantc/codex-ai/releases/download/v0.5.9/Codex.AI.System.app.tar.gz",
    );
  });
});
