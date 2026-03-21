#!/usr/bin/env node
/**
 * Generate Tauri app icons from the Production Point SVG logo.
 * Creates PNGs with a dark background at all required sizes.
 */
const sharp = require("sharp");
const path = require("path");
const fs = require("fs");

const ROOT = path.resolve(__dirname, "..");
const ICONS_DIR = path.join(ROOT, "src-tauri", "icons");
const SVG_PATH = path.join(ICONS_DIR, "pp-logo.svg");

// Read SVG and add a dark rounded-rect background
const svgContent = fs.readFileSync(SVG_PATH, "utf8");

// Create a composite SVG with dark background and the logo centered with padding
function createIconSvg(size) {
  const padding = Math.round(size * 0.15);
  const logoSize = size - padding * 2;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
  <rect width="${size}" height="${size}" rx="${Math.round(size * 0.18)}" fill="#1a1d26"/>
  <svg x="${padding}" y="${padding}" width="${logoSize}" height="${logoSize}" viewBox="0 0 1264.71 1514.79">
    <defs><style>.cls-1{fill:#fff}.cls-2{fill:#dc4c3e}</style></defs>
    <path class="cls-1" d="m857.44,632.35c0,35.09-7.95,68.76-23.64,100.07-3.56,7.1-7.52,14.02-11.83,20.73,5.88,6.01,11.81,11.99,17.8,17.89,6.09,6,12.24,11.93,18.44,17.82,7.58-10.86,14.38-22.24,20.29-34.03,19.2-38.33,28.93-79.53,28.93-122.46,0-73.2-28.69-142.21-80.79-194.3-52.1-52.1-121.1-80.79-194.3-80.79s-142.21,28.69-194.3,80.79c-52.1,52.1-80.79,121.1-80.79,194.3v678.81c0,40.84-16.04,79.37-45.15,108.48-29.12,29.12-67.64,45.15-108.48,45.15s-79.37-16.04-108.48-45.15c-29.12-29.12-45.15-67.64-45.15-108.48v-678.81c0-78.74,15.38-155.06,45.7-226.85,29.3-69.36,71.26-131.63,124.72-185.09,53.46-53.46,115.73-95.42,185.09-124.72,71.79-30.33,148.11-45.7,226.85-45.7s155.06,15.38,226.85,45.7c69.36,29.3,131.63,71.26,185.09,124.72,53.46,53.46,95.42,115.73,124.72,185.09,30.33,71.79,45.7,148.11,45.7,226.85,0,105.16-28.23,208.16-81.63,297.84-10.84,18.21-22.67,35.76-35.39,52.61,5.83,4.23,11.66,8.46,17.5,12.68,7.68,5.54,15.37,11.07,23.06,16.59,13.56-18.04,26.19-36.83,37.78-56.3,58.01-97.42,88.67-209.25,88.67-323.42,0-85.46-16.7-168.33-49.64-246.31-31.82-75.33-77.38-142.95-135.42-200.99-58.04-58.04-125.66-103.6-200.99-135.42C800.68,16.7,717.81,0,632.35,0s-168.33,16.7-246.31,49.64c-75.33,31.82-142.95,77.38-200.99,135.42-58.04,58.04-103.6,125.66-135.42,200.99C16.7,464.03,0,546.9,0,632.35v678.81c0,54.2,21.24,105.28,59.8,143.84,38.56,38.56,89.64,59.8,143.84,59.8s105.28-21.24,143.84-59.8c38.56-38.56,59.8-89.64,59.8-143.84v-678.81c0-59.85,23.49-116.3,66.14-158.95,42.65-42.65,99.1-66.14,158.95-66.14s116.29,23.49,158.95,66.14c42.65,42.65,66.14,99.1,66.14,158.95Z"/>
    <circle class="cls-2" cx="858.73" cy="1040.6" r="176.64" transform="translate(-484.3 912) rotate(-45)"/>
  </svg>
</svg>`;
}

async function generate() {
  const sizes = [
    { name: "32x32.png", size: 32 },
    { name: "128x128.png", size: 128 },
    { name: "128x128@2x.png", size: 256 },
    { name: "icon.png", size: 512 },
  ];

  for (const { name, size } of sizes) {
    const svg = createIconSvg(size);
    await sharp(Buffer.from(svg))
      .png()
      .toFile(path.join(ICONS_DIR, name));
    console.log(`  ✓ ${name} (${size}x${size})`);
  }

  // Generate ICO from the 256px PNG (Windows icon)
  // sharp doesn't support ICO directly, but Tauri can use the PNG
  // We'll create a simple ICO by just copying the 256px as the main icon
  const png256 = await sharp(Buffer.from(createIconSvg(256))).png().toBuffer();
  fs.writeFileSync(path.join(ICONS_DIR, "icon.ico"), png256);
  console.log("  ✓ icon.ico (256x256 PNG wrapped)");

  console.log("\nIcons generated from Production Point logo.");
}

generate().catch(console.error);
