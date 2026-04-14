#!/usr/bin/env node
/**
 * One-command release: bump version, commit, tag, push.
 * GitHub Actions CI then builds the installer and publishes the release.
 *
 * Usage:
 *   node scripts/release.cjs           # patch bump (0.2.0 → 0.2.1)
 *   node scripts/release.cjs minor     # minor bump (0.2.0 → 0.3.0)
 *   node scripts/release.cjs major     # major bump (0.2.0 → 1.0.0)
 */
const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const bumpType = process.argv[2] || "patch";

function run(cmd) {
  console.log(`  $ ${cmd}`);
  return execSync(cmd, { cwd: root, stdio: "inherit" });
}

// 1. Bump version
console.log("\n1. Bumping version...");
run(`node scripts/bump-version.cjs ${bumpType}`);

// Read new version
const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const version = pkg.version;
const tag = `v${version}`;

// 2. Stage and commit
console.log("\n2. Committing...");
run("git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock");
run(`git commit -m "release: ${tag}"`);

// 3. Tag
console.log("\n3. Tagging...");
run(`git tag ${tag}`);

// 4. Push
console.log("\n4. Pushing...");
run("git push origin main");
run(`git push origin ${tag}`);

console.log(`
Done! Release ${tag} pushed.
GitHub Actions will now build the installer and publish the release.
Track progress: https://github.com/production-point/flow-tasks/actions
`);
