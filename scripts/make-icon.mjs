// Generates icon-source.png (512x512) for the app. Pure stdlib: writes a valid
// PNG by hand (zlib + CRC32), so no image dependencies are needed.
// Usage: bun scripts/make-icon.mjs   (then: bunx tauri icon icons/icon-source.png)
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 512;

// --- CRC32 (PNG chunk checksums) ---
const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  crcTable[n] = c >>> 0;
}
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

// --- scene ---
// Emerald rounded-square tile + bold white rounded play triangle.
// High contrast and thick strokes so it stays readable at 32px in the taskbar.
function roundRectDist(px, py, half, radius) {
  const qx = Math.abs(px) - half + radius;
  const qy = Math.abs(py) - half + radius;
  const ax = Math.max(qx, 0);
  const ay = Math.max(qy, 0);
  return Math.hypot(ax, ay) + Math.min(Math.max(qx, qy), 0) - radius;
}

function inTriangle(x, y, tri) {
  const [a, b, c] = tri;
  const s = (p, q, r) => (p[0] - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p[1] - r[1]);
  const d1 = s([x, y], a, b);
  const d2 = s([x, y], b, c);
  const d3 = s([x, y], c, a);
  return !(d1 < 0 || d2 < 0 || d3 < 0) || !(d1 > 0 || d2 > 0 || d3 > 0);
}

function distToSegment(px, py, a, b) {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const t = Math.max(0, Math.min(1, ((px - a[0]) * dx + (py - a[1]) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(px - (a[0] + t * dx), py - (a[1] + t * dy));
}

// Triangle vertices (visual center of mass nudged right of geometric center).
const TRI = [
  [214, 176],
  [214, 336],
  [356, 256],
];
const TRI_ROUND = 18;

function roundedTriangle(x, y) {
  if (inTriangle(x, y, TRI)) return true;
  for (const v of TRI) {
    if (Math.hypot(x - v[0], y - v[1]) <= TRI_ROUND) return true;
  }
  for (let i = 0; i < 3; i++) {
    if (distToSegment(x, y, TRI[i], TRI[(i + 1) % 3]) <= TRI_ROUND) return true;
  }
  return false;
}

function sample(x, y) {
  const half = SIZE / 2;
  const tile = roundRectDist(x - half, y - half, half, 112);
  if (tile > 0.7) return null;

  // Tile: vertical emerald gradient with a slightly brighter top rim.
  const t = y / SIZE;
  let r = 59 + (23 - 59) * t;
  let g = 224 + (178 - 224) * t;
  let b = 138 + (106 - 138) * t;
  if (tile > -22) {
    // 1.5% brighter rim right inside the edge
    r *= 1.12;
    g *= 1.12;
    b *= 1.12;
  }

  if (roundedTriangle(x, y)) {
    r = 255;
    g = 255;
    b = 255;
  }
  return [Math.min(r, 255), Math.min(g, 255), Math.min(b, 255), 255];
}

// --- render with 4x4 supersampling ---
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
let off = 0;
for (let y = 0; y < SIZE; y++) {
  raw[off++] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    let r = 0, g = 0, b = 0, a = 0;
    for (let sy = 0; sy < 4; sy++) {
      for (let sx = 0; sx < 4; sx++) {
        const s = sample(x + (sx + 0.5) / 4, y + (sy + 0.5) / 4);
        if (!s) continue;
        r += s[0]; g += s[1]; b += s[2]; a += s[3];
      }
    }
    raw[off++] = Math.round(r / 16);
    raw[off++] = Math.round(g / 16);
    raw[off++] = Math.round(b / 16);
    raw[off++] = Math.round(a / 16);
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8;  // bit depth
ihdr[9] = 6;  // color type RGBA
ihdr[10] = 0;
ihdr[11] = 0;
ihdr[12] = 0;

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
mkdirSync(join(root, "icons"), { recursive: true });
const out = join(root, "icons", "icon-source.png");
writeFileSync(out, png);
console.log(`wrote ${out} (${png.length} bytes)`);
