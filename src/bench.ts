/** 验收基准模式（doc/04 性能要求 §3）：
 *  冷缓存运行（--bench-clear）：P2 扫描、P3 首 60 张出图、P4 滚动帧率
 *  热缓存运行（无 --bench-clear）：P5 二次打开出图 */
import { ipc, type Entry } from "./ipc";
import type { Grid } from "./grid/grid";
import type { BenchConfig } from "./ipc";

const P3_TARGET = 60; // 前 60 张可见缩略图
const P5_TARGET = 12;
const LOAD_TIMEOUT_MS = 20000;
const SCROLL_TEST_MS = 8000;
const SCROLL_SPEED_PX_S = 2500;

export async function runBench(
  cfg: BenchConfig,
  grid: Grid,
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
  }

  if (cfg.exit) {
    await ipc.benchDone();
  }
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
