/** 生成应用图标源图 scripts/icon-src.png（1024×1024 RGBA）
 *  纯 Node 实现（zlib 内置），无第三方依赖；之后用 `pnpm tauri icon` 生成全套。 */
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const S = 1024;
const OUT = join(dirname(fileURLToPath(import.meta.url)), "icon-src.png");

// ── PNG 编码基础件 ─────────────────────────────
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}

// ── 画面：靛蓝→紫渐变底 + 镜头圆环 + 高光点 ─────────
const px = Buffer.alloc(S * S * 4);
const c0 = [79, 70, 229]; // indigo
const c1 = [147, 51, 234]; // violet
const CX = S / 2;
const CY = S / 2;
const R = 280;
const TH = 52;
const HD_X = 410;
const HD_Y = 400;
const HD_R = 64;

for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    const t = (x + y) / (2 * S);
    let r = c0[0] + (c1[0] - c0[0]) * t;
    let g = c0[1] + (c1[1] - c0[1]) * t;
    let b = c0[2] + (c1[2] - c0[2]) * t;

    const d = Math.hypot(x - CX, y - CY);
    const ring = 1 - Math.min(1, Math.abs(d - R) / (TH / 2)); // 1=环心 0=环外
    if (ring > 0) {
      const a = Math.min(1, ring * 3); // 环边缘抗锯齿
      r = r * (1 - a) + 245 * a;
      g = g * (1 - a) + 243 * a;
      b = b * (1 - a) + 255 * a;
    } else if (d < R) {
      // 镜片内淡淡的亮色
      const a = 0.14;
      r = r * (1 - a) + 255 * a;
      g = g * (1 - a) + 255 * a;
      b = b * (1 - a) + 255 * a;
    }

    const hd = Math.hypot(x - HD_X, y - HD_Y);
    if (hd < HD_R) {
      const a = (1 - hd / HD_R) * 0.85;
      r = r * (1 - a) + 255 * a;
      g = g * (1 - a) + 255 * a;
      b = b * (1 - a) + 255 * a;
    }

    const i = (y * S + x) * 4;
    px[i] = Math.round(r);
    px[i + 1] = Math.round(g);
    px[i + 2] = Math.round(b);
    px[i + 3] = 255;
  }
}

// ── 组装 PNG ─────────────────────────────
const raw = Buffer.alloc(S * (S * 4 + 1));
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0; // filter: None
  px.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

writeFileSync(OUT, png);
console.log(`icon 源图已生成: ${OUT} (${png.length} bytes)`);
