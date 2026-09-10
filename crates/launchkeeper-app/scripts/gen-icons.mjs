// 生成 macOS 菜单栏模板图标，不依赖任何图形库：
// 直接计算像素覆盖率写 PNG，zlib 用 Node 内置的。
//
//   node scripts/gen-icons.mjs
//
// 产出：
//   src-tauri/icons/tray-ok.png       22x22 守护环 + 勾
//   src-tauri/icons/tray-failed.png   22x22 守护环 + 感叹号
//   src-tauri/icons/tray-running.png  22x22 守护环 + 播放符号
//
// 三张 tray 图是模板图标（纯黑 + alpha），tauri.conf 里 iconAsTemplate=true
// 时 macOS 会按菜单栏明暗自动反色，所以这里不能用彩色。
// 应用图标主源图单独保存在 src-tauri/icon-source.png，本脚本不会覆盖它。

import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const iconsDir = join(appDir, "src-tauri", "icons");

/** 每个像素每轴的采样数；4x4 足够让圆角和斜线看不出锯齿。 */
const SS = 4;

// ---------------------------------------------------------------- 形状谓词

const roundedRect = (x0, y0, x1, y1, r) => (x, y) => {
  if (x < x0 || x > x1 || y < y0 || y > y1) return false;
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
};

const disc = (cx, cy, r) => (x, y) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;

const ring = (cx, cy, r, w) => (x, y) => {
  const d = Math.hypot(x - cx, y - cy);
  return d <= r && d >= r - w;
};

/** 粗细为 w 的线段（含圆头），用点到线段的距离判定。 */
const segment = (ax, ay, bx, by, w) => (x, y) => {
  const dx = bx - ax;
  const dy = by - ay;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.min(1, Math.max(0, ((x - ax) * dx + (y - ay) * dy) / len2));
  return Math.hypot(x - (ax + t * dx), y - (ay + t * dy)) <= w / 2;
};

const triangle = (ax, ay, bx, by, cx, cy) => {
  const side = (px, py, qx, qy, rx, ry) => (qx - px) * (ry - py) - (qy - py) * (rx - px);
  return (x, y) => {
    const s1 = side(ax, ay, bx, by, x, y);
    const s2 = side(bx, by, cx, cy, x, y);
    const s3 = side(cx, cy, ax, ay, x, y);
    return (s1 >= 0 && s2 >= 0 && s3 >= 0) || (s1 <= 0 && s2 <= 0 && s3 <= 0);
  };
};

const union =
  (...fns) =>
  (x, y) =>
    fns.some((f) => f(x, y));

const minus = (a, b) => (x, y) => a(x, y) && !b(x, y);

// ---------------------------------------------------------------- 光栅化

/** 画布：RGBA，初始全透明。 */
function canvas(size) {
  return { size, px: new Uint8Array(size * size * 4) };
}

/** 用超采样求覆盖率，把 [r,g,b] 以 alpha=coverage 合成（source-over）。 */
function fill(cv, shape, [r, g, b]) {
  const { size, px } = cv;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let hits = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          if (shape(x + (sx + 0.5) / SS, y + (sy + 0.5) / SS)) hits++;
        }
      }
      if (hits === 0) continue;
      const a = hits / (SS * SS);
      const i = (y * size + x) * 4;
      const dst = px[i + 3] / 255;
      const out = a + dst * (1 - a);
      // 预乘再解预乘，否则半透明边缘的颜色会偏。
      px[i] = Math.round((r * a + px[i] * dst * (1 - a)) / out);
      px[i + 1] = Math.round((g * a + px[i + 1] * dst * (1 - a)) / out);
      px[i + 2] = Math.round((b * a + px[i + 2] * dst * (1 - a)) / out);
      px[i + 3] = Math.round(out * 255);
    }
  }
}

// ---------------------------------------------------------------- PNG 编码

function crc32(buf) {
  let c = ~0;
  for (const byte of buf) {
    c ^= byte;
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])), 0);
  return Buffer.concat([head, data, crc]);
}

function encodePng({ size, px }) {
  // 每行前面加一个 filter 字节（0 = None）。
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    Buffer.from(px.buffer, y * size * 4, size * 4).copy(raw, y * (size * 4 + 1) + 1);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function write(path, cv) {
  writeFileSync(path, encodePng(cv));
  console.log(`${path} (${cv.size}x${cv.size})`);
}

// ---------------------------------------------------------------- tray 图标

const BLACK = [0, 0, 0];

function trayOk(size) {
  const cv = canvas(size);
  const u = size / 22;
  // 开口的“守护环”与应用图标同源；勾的末端同时补上圆环缺口。
  const orbit = ring(10.3 * u, 10.6 * u, 7.25 * u, 2.2 * u);
  const gap = (x, y) => x > 12.1 * u && y > 11.1 * u;
  const check = union(
    segment(11.7 * u, 14.1 * u, 14.2 * u, 16.3 * u, 2.35 * u),
    segment(14.2 * u, 16.3 * u, 19 * u, 10.4 * u, 2.35 * u),
  );
  fill(cv, union(minus(orbit, gap), check), BLACK);
  return cv;
}

function trayFailed(size) {
  const cv = canvas(size);
  const u = size / 22;
  const orbit = ring(11 * u, 11 * u, 8 * u, 2.15 * u);
  const bar = segment(11 * u, 6.6 * u, 11 * u, 11.9 * u, 2.3 * u);
  const dot = disc(11 * u, 15 * u, 1.2 * u);
  fill(cv, union(orbit, bar, dot), BLACK);
  return cv;
}

function trayRunning(size) {
  const cv = canvas(size);
  const u = size / 22;
  const orbit = ring(11 * u, 11 * u, 8 * u, 2.15 * u);
  const play = triangle(9.5 * u, 7.4 * u, 9.5 * u, 14.6 * u, 15.4 * u, 11 * u);
  fill(cv, union(orbit, play), BLACK);
  return cv;
}

// ---------------------------------------------------------------- main

mkdirSync(iconsDir, { recursive: true });
write(join(iconsDir, "tray-ok.png"), trayOk(22));
write(join(iconsDir, "tray-failed.png"), trayFailed(22));
write(join(iconsDir, "tray-running.png"), trayRunning(22));
// macOS 菜单栏在 Retina 上按 @2x 取图；缺了它图标会被放大成糊的。
write(join(iconsDir, "tray-ok@2x.png"), trayOk(44));
write(join(iconsDir, "tray-failed@2x.png"), trayFailed(44));
write(join(iconsDir, "tray-running@2x.png"), trayRunning(44));

console.log("\n如果应用图标主源图有变化，再跑: pnpm tauri icon src-tauri/icon-source.png");
