#!/usr/bin/env node
/**
 * Bumps the patch version across package.json, src-tauri/Cargo.toml, and src-tauri/tauri.conf.json.
 * Usage: node scripts/bump-version.js [major|minor|patch]
 * Default: patch
 */
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const bumpType = process.argv[2] || "patch";

// Read current version from package.json
const pkgPath = path.join(root, "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
const [major, minor, patch] = pkg.version.split(".").map(Number);

let newVersion;
if (bumpType === "major") newVersion = `${major + 1}.0.0`;
else if (bumpType === "minor") newVersion = `${major}.${minor + 1}.0`;
else newVersion = `${major}.${minor}.${patch + 1}`;

console.log(`Bumping version: ${pkg.version} → ${newVersion}`);

// Update package.json
pkg.version = newVersion;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log("  ✓ package.json");

// Update tauri.conf.json
const tauriConfPath = path.join(root, "src-tauri", "tauri.conf.json");
const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, "utf8"));
tauriConf.version = newVersion;
fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + "\n");
console.log("  ✓ tauri.conf.json");

// Update Cargo.toml
const cargoPath = path.join(root, "src-tauri", "Cargo.toml");
let cargo = fs.readFileSync(cargoPath, "utf8");
cargo = cargo.replace(/^version = ".*"/m, `version = "${newVersion}"`);
fs.writeFileSync(cargoPath, cargo);
console.log("  ✓ Cargo.toml");

console.log(`\nVersion bumped to ${newVersion}`);
console.log(`Run: git tag v${newVersion} && git push origin v${newVersion}`);
