/** 验收基准模式（doc/04 性能要求 §3）：
 *  冷缓存运行（--bench-clear）：P2 扫描、P3 首 60 张出图、P4 滚动帧率、P6 翻页、P7 大图平移
 *  热缓存运行（无 --bench-clear）：P5 二次打开出图 */
import { ipc, type Entry } from "./ipc";
import type { Grid } from "./grid/grid";
import type { BenchConfig } from "./ipc";
import type { Viewer } from "./viewer/viewer";

const P3_TARGET = 60; // 前 60 张可见缩略图
const P5_TARGET = 12;
const LOAD_TIMEOUT_MS = 20000;
const SCROLL_TEST_MS = 8000;
const SCROLL_SPEED_PX_S = 2500;

export async function runBench(
  cfg: BenchConfig,
  grid: Grid,
  viewer: Viewer,
  loadEntries: (entries: Entry[]) => void,
  hooks: Set<(e: Entry) => void>,
): Promise<void> {
  const log = (metric: string, value: number) => ipc.logBench(metric, value);

  if (cfg.clear) {
    await ipc.benchClearCache();
  }

  const res = await ipc.scanFolder(cfg.folder);
  loadEntries(res.entries);
  log(cfg.clear ? "P2_scan_cold_ms" : "P2_scan_warm_ms", res.elapsedMs);

  const tScanDone = performance.now();
  const target = cfg.clear
    ? Math.min(P3_TARGET, res.entries.length)
    : Math.min(P5_TARGET, res.entries.length);
  let loaded = 0;
  await new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      hooks.delete(onLoad);
      const ms = performance.now() - tScanDone;
      log(cfg.clear ? "P3_first60_cold_ms" : "P5_first12_warm_ms", ms);
      resolve();
    };
    const onLoad = () => {
      loaded++;
      if (loaded >= target) finish();
    };
    const timer = setTimeout(finish, LOAD_TIMEOUT_MS);
    hooks.add(onLoad);
  });

  if (cfg.clear) {
    await scrollBench(grid, log);
    await viewerBench(viewer, res.entries, log);
  }

  if (cfg.exit) {
    await ipc.benchDone();
  }
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** P6 翻页响应（预取 ±2）+ P7 大图 100% 平移帧率 */
async function viewerBench(
  viewer: Viewer,
  entries: Entry[],
  log: (m: string, v: number) => void,
): Promise<void> {
  if (entries.length < 4) return;
  const mid = entries[Math.floor(entries.length / 2)]!;
  viewer.open(mid.path);
  await sleep(2000); // 等预取稳定

  // P6a：预取命中翻页 ×10
  const pre: number[] = [];
  for (let i = 0; i < 10; i++) {
    pre.push(await timedNext(viewer, 1));
    await sleep(350);
  }
  log("P6_prefetched_avg_ms", avg(pre));
  log("P6_prefetched_max_ms", Math.max(...pre));

  // P6b：跳出预取窗口（±9）×3 —— 未预取
  const cold: number[] = [];
  for (let i = 0; i < 3; i++) {
    cold.push(await timedJump(viewer, 9));
    await sleep(700);
  }
  log("P6_coldjump_avg_ms", avg(cold));
  log("P6_coldjump_max_ms", Math.max(...cold));

  // P7：图库中存在大图（>5MB）时，100% 缩放平移 8s
  const big = entries.reduce((a, b) => (b.size > a.size ? b : a));
  if (big.size > 5 * 1024 * 1024) {
    await sleep(500);
    viewer.open(big.path);
    await sleep(2500); // 等预览 + 全尺寸升级
    const target = viewer.current() ?? big;
    await viewer.benchWaitFull(target);
    await sleep(300);
    await viewer.benchPan(SCROLL_TEST_MS, log);
  }
  viewer.close();
}

function timedNext(viewer: Viewer, dir: 1 | -1): Promise<number> {
  return timeDraw(viewer, viewer.peekNext(dir), () => viewer.next(dir));
}

function timedJump(viewer: Viewer, n: number): Promise<number> {
  // 跳出预取窗口：以翻页后第一帧绘制为准（无预期目标）
  return timeDraw(viewer, null, () => viewer.jump(n));
}

function timeDraw(
  viewer: Viewer,
  expect: Entry | null,
  fire: () => void,
): Promise<number> {
  return new Promise((resolve) => {
    const t0 = performance.now();
    let done = false;
    const orig = viewer.onDrawn;
    const onDraw = (e: Entry) => {
      if (done) return;
      if (expect && e.path !== expect.path) return;
      done = true;
      viewer.onDrawn = orig;
      resolve(performance.now() - t0);
    };
    viewer.onDrawn = onDraw;
    fire();
    // 超时保护：2s 未绘制则放弃该样本
    setTimeout(() => {
      if (!done) {
        done = true;
        viewer.onDrawn = orig;
        resolve(9999);
      }
    }, 2000);
  });
}

function avg(a: number[]): number {
  return a.length ? a.reduce((s, x) => s + x, 0) / a.length : 0;
}

/** 程序化滚动 8 秒，统计平均帧率与最差 1 秒窗口帧率（P4） */
function scrollBench(grid: Grid, log: (m: string, v: number) => void): Promise<void> {
  return new Promise((resolve) => {
    const pxPerFrame = SCROLL_SPEED_PX_S / 60;
    const start = performance.now();
    let frames = 0;
    let winFrames = 0;
    let winStart = start;
    let minFps = Infinity;
    const step = () => {
      const now = performance.now();
      if (now - start >= SCROLL_TEST_MS) {
        log("P4_scroll_avg_fps", frames / ((now - start) / 1000));
        log("P4_scroll_min_1s_fps", minFps === Infinity ? 0 : minFps);
        resolve();
        return;
      }
      grid.scrollBy(pxPerFrame);
      frames++;
      winFrames++;
      if (now - winStart >= 1000) {
        minFps = Math.min(minFps, winFrames);
        winFrames = 0;
        winStart = now;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
}
