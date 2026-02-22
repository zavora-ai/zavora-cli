#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");

const binaryPath = path.join(__dirname, "..", "vendor", "zavora-cli");

if (!fs.existsSync(binaryPath)) {
  console.error("zavora-cli binary is not installed. Reinstall with `npm i -g @zavora-ai/zavora-cli`.");
  process.exit(1);
}

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit"
});

child.on("error", (error) => {
  console.error(`failed to run zavora-cli: ${error.message}`);
  process.exit(1);
});

child.on("exit", (code) => {
  process.exit(code === null ? 1 : code);
});
