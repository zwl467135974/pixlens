/** 验收基准模式（doc/04 性能要求 §3）：
 *  冷缓存运行（--bench-clear）：P2 扫描、P3 首 60 张出图、P4 滚动帧率、P6 翻页、P7 大图平移、
 *  P8 PSD/PSB、M4 TIFF/HDR、M5 批量转换
 *  热缓存运行（无 --bench-clear）：P5 二次打开出图 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipc, thumbUrl, imageUrl, type BatchProgress, type Entry } from "./ipc";
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
    await viewerBench(cfg.folder, viewer, res.entries, log);
  }

  if (cfg.exit) {
    await ipc.benchDone();
  }
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** P6 翻页响应（预取 ±2）+ P7 大图 100% 平移帧率 */
async function viewerBench(
  folder: string,
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

  // P8：PSD/PSB 专项（存在时）——按大小取前 3（优先巨型文件）
  const psds = entries
    .filter((e) => e.ext === "psd" || e.ext === "psb")
    .sort((a, b) => b.size - a.size)
    .slice(0, 3);
  for (const e of psds) {
    const tag = e.name.replace(/[^A-Za-z0-9_\u4e00-\u9fa5]+/g, "_");
    let t0 = performance.now();
    const r1 = await fetch(thumbUrl(e, 256));
    await r1.arrayBuffer();
    log(`P8_thumb_${tag}_ms`, performance.now() - t0);
    t0 = performance.now();
    const r2 = await fetch(imageUrl(e, 2560));
    await r2.arrayBuffer();
    log(`P8_preview_${tag}_ms`, performance.now() - t0);
  }

  // M4a：多页 TIFF 页间切换（X-PixLens-Pages 检出多页后测 1..3 页延迟）
  for (const e of entries.filter((x) => x.ext === "tif" || x.ext === "tiff")) {
    const tag = e.name.replace(/[^A-Za-z0-9_\u4e00-\u9fa5]+/g, "_");
    const r0 = await fetch(imageUrl(e, 2560, 0, 1));
    const pgs = Number(r0.headers.get("X-PixLens-Pages") ?? "1");
    await r0.arrayBuffer();
    if (pgs > 1) {
      const lat: number[] = [];
      for (let p = 1; p < Math.min(pgs, 4); p++) {
        const t = performance.now();
        const r = await fetch(imageUrl(e, 2560, p, 1));
        await r.arrayBuffer();
        lat.push(performance.now() - t);
      }
      log(`M4_tiff_pages_${tag}`, pgs);
      log("M4_tiff_page_switch_avg_ms", avg(lat));
      log("M4_tiff_page_switch_max_ms", lat.length ? Math.max(...lat) : 0);
      // 热缓存二次切换（磁盘缓存命中路径）
      const tw = performance.now();
      const rw = await fetch(imageUrl(e, 2560, 1, 1));
      await rw.arrayBuffer();
      log("M4_tiff_page_switch_warm_ms", performance.now() - tw);
      break; // 测第一个多页文件即可
    }
  }

  // M4b：HDR 曝光可调（预览 + 换曝光重解码，字节应有差异）
  for (const e of entries.filter((x) => x.ext === "hdr")) {
    const tag = e.name.replace(/[^A-Za-z0-9_\u4e00-\u9fa5]+/g, "_");
    let t = performance.now();
    const r1 = await fetch(imageUrl(e, 2560, 0, 1));
    const b1 = await r1.arrayBuffer();
    log(`M4_hdr_preview_${tag}_ms`, performance.now() - t);
    t = performance.now();
    const r2 = await fetch(imageUrl(e, 2560, 0, 4));
    const b2 = await r2.arrayBuffer();
    log("M4_hdr_exposure_change_ms", performance.now() - t);
    log(`M4_hdr_exposure_differs_${tag}`, b1.byteLength !== b2.byteLength ? 1 : 0);
    break;
  }

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

  // M5：批量转换（全量可转文件）——吞吐 + UI 帧率 + 进度准确 + 取消
  await batchBench(folder, entries, log);

  // M6：编辑落盘（另存副本，原图保留由单元测试保证字节不变）
  const m6 = entries.find((e) => e.ext === "jpg");
  if (m6) {
    try {
      const out = await ipc.editApply(
        m6.path,
        { crop: null, rotate: 90, flipH: true, flipV: false, brightness: 15, contrast: 10, saturation: -20, filter: "sepia" },
        false,
        90,
      );
      log("M6_edit_saved", out ? 1 : 0);
    } catch {
      log("M6_edit_failed", 1);
    }
  }
}

/** M5 验收：1 万张批量转换无卡死、可取消、进度准确 */
async function batchBench(
  folder: string,
  entries: Entry[],
  log: (m: string, v: number) => void,
): Promise<void> {
  const convExt = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "tif", "tiff"]);
  const conv = entries.filter((e) => convExt.has(e.ext));
  if (conv.length < 500) return;
  const outDir = folder.replace(/[\\/][^\\/]+$/, "") + "\\bench_out";

  const events: BatchProgress[] = [];
  const un: UnlistenFn = await listen<BatchProgress>("batch-progress", (ev) => events.push(ev.payload));

  // UI 帧率监控（转换期间主线程不应被阻塞）
  let frames = 0;
  let fpsStop = false;
  const raf = () => {
    if (fpsStop) return;
    frames++;
    requestAnimationFrame(raf);
  };
  requestAnimationFrame(raf);

  const t0 = performance.now();
  const jobId = await ipc.batchConvert(
    conv.map((e) => e.path),
    { format: "jpg", quality: 80, scaleMode: "percent", scaleValue: 30, outPolicy: "subdir", outDir },
  );
  const waitFinish = async (id: number, timeoutMs: number): Promise<BatchProgress | null> => {
    const start = performance.now();
    for (;;) {
      const fin = events.find((e) => e.jobId === id && e.finished);
      if (fin) return fin;
      if (performance.now() - start > timeoutMs) return null;
      await sleep(100);
    }
  };
  const fin = await waitFinish(jobId, 300_000);
  fpsStop = true;
  if (fin) {
    log("M5_convert_total_ms", performance.now() - t0);
    log("M5_convert_count", conv.length);
    log("M5_convert_done", fin.done);
    log("M5_convert_failed", fin.failed);
    log("M5_progress_accurate", fin.done + fin.failed === conv.length ? 1 : 0);
  } else {
    log("M5_convert_timeout", 1);
  }
  log("M5_convert_ui_fps_avg", frames / ((performance.now() - t0) / 1000));

  // 取消测试：再提交 2000 张，600ms 后取消
  const j2 = await ipc.batchConvert(conv.slice(0, 2000).map((e) => e.path), {
    format: "jpg",
    quality: 80,
    scaleMode: "percent",
    scaleValue: 30,
    outPolicy: "subdir",
    outDir,
  });
  await sleep(600);
  await ipc.batchCancel(j2);
  const fin2 = await waitFinish(j2, 60_000);
  if (fin2) {
    log("M5_cancel_ok", fin2.canceled ? 1 : 0);
    log("M5_cancel_done_before_stop", fin2.done);
  }
  un();
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
