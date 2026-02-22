#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const https = require("node:https");
const { execFileSync } = require("node:child_process");

const SUPPORTED_ARTIFACTS = {
  linux: {
    x64: "linux-x64",
    arm64: "linux-arm64"
  },
  darwin: {
    x64: "darwin-x64",
    arm64: "darwin-arm64"
  }
};

function fail(message) {
  console.error(`zavora-cli npm install failed: ${message}`);
  console.error("Fallback: install with `cargo install zavora-cli`.");
  process.exit(1);
}

function downloadFile(url, destination) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(destination);

    const request = https.get(url, (response) => {
      if (
        response.statusCode &&
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location
      ) {
        file.close();
        fs.rmSync(destination, { force: true });
        downloadFile(response.headers.location, destination)
          .then(resolve)
          .catch(reject);
        return;
      }

      if (response.statusCode !== 200) {
        file.close();
        fs.rmSync(destination, { force: true });
        reject(new Error(`HTTP ${response.statusCode} from ${url}`));
        return;
      }

      response.pipe(file);
      file.on("finish", () => {
        file.close();
        resolve();
      });
    });

    request.on("error", (error) => {
      file.close();
      fs.rmSync(destination, { force: true });
      reject(error);
    });
  });
}

function readPackageVersion() {
  const pkgPath = path.join(__dirname, "..", "package.json");
  const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
  return pkg.version;
}

function resolveArtifactSuffix() {
  const platformMap = SUPPORTED_ARTIFACTS[process.platform];
  if (!platformMap) {
    return null;
  }
  return platformMap[process.arch] ?? null;
}

function parseExpectedChecksum(checksumsText, assetName) {
  for (const rawLine of checksumsText.split("\n")) {
    const line = rawLine.trim();
    if (!line) {
      continue;
    }
    const parts = line.split(/\s+/);
    if (parts.length < 2) {
      continue;
    }
    const checksum = parts[0];
    const filename = parts[parts.length - 1].replace(/^\*/, "");
    if (filename === assetName) {
      return checksum;
    }
  }
  return null;
}

function sha256ForFile(filePath) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

function unpackArchive(archivePath, destination) {
  fs.rmSync(destination, { recursive: true, force: true });
  fs.mkdirSync(destination, { recursive: true });
  execFileSync("tar", ["-xzf", archivePath, "-C", destination], { stdio: "inherit" });
}

async function main() {
  const dryRun = process.argv.includes("--dry-run");
  const artifactSuffix = resolveArtifactSuffix();
  if (!artifactSuffix) {
    fail(`unsupported platform/arch combination: ${process.platform}/${process.arch}`);
  }

  const version = readPackageVersion();
  const tag = `v${version}`;
  const assetName = `zavora-cli-${tag}-${artifactSuffix}.tar.gz`;
  const baseUrl = `https://github.com/zavora-ai/zavora-cli/releases/download/${tag}`;
  const archiveUrl = `${baseUrl}/${assetName}`;

  if (dryRun) {
    console.log(`[dry-run] ${archiveUrl}`);
    return;
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "zavora-cli-"));
  const archivePath = path.join(tempDir, assetName);
  const checksumsPath = path.join(tempDir, "checksums.txt");

  try {
    console.log(`Downloading ${assetName}...`);
    await downloadFile(archiveUrl, archivePath);

    try {
      await downloadFile(`${baseUrl}/checksums.txt`, checksumsPath);
      const checksumsText = fs.readFileSync(checksumsPath, "utf8");
      const expected = parseExpectedChecksum(checksumsText, assetName);
      if (expected) {
        const actual = sha256ForFile(archivePath);
        if (actual !== expected) {
          fail(`checksum mismatch for ${assetName}`);
        }
      }
    } catch {
      // Continue if checksum manifest is unavailable.
    }

    const vendorDir = path.join(__dirname, "..", "vendor");
    unpackArchive(archivePath, vendorDir);

    const binaryPath = path.join(vendorDir, "zavora-cli");
    if (!fs.existsSync(binaryPath)) {
      fail(`archive ${assetName} did not contain zavora-cli binary`);
    }
    fs.chmodSync(binaryPath, 0o755);
  } catch (error) {
    fail(error.message);
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

main();
