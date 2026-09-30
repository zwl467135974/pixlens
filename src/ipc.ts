/** Tauri IPC 封装与 thumb:// URL 构造（doc/03-架构设计 §4） */
import { invoke } from "@tauri-apps/api/core";

export interface Entry {
  name: string;
  path: string;
  ext: string;
  size: number;
  mtime: number;
  /** "image" 走图片管线；"video" 走 MF 首帧缩略图 + 系统播放器 */
  kind: "image" | "video";
}

export interface ScanResult {
  entries: Entry[];
  elapsedMs: number;
}

export interface FsChanged {
  created: Entry[];
  removed: string[];
  updated: Entry[];
}

export interface BenchConfig {
  folder: string;
  clear: boolean;
  exit: boolean;
  full: boolean;
}

export interface AppSettings {
  cacheLimitMb: number;
  defaultSort: string;
  theme: string;
  /** Rust 侧维护（窗口几何/上次文件夹），前端只读 */
  window?: unknown;
  lastFolder?: string | null;
  /** Explorer 缓存提示横幅是否已展示（每台机器一次） */
  thumbHintShown?: boolean;
}

export interface RenamePair {
  path: string;
  name: string;
  newName: string;
  conflict: boolean;
  reason: string;
}

export interface ConvertOptions {
  format: string;
  quality: number;
  scaleMode: string;
  scaleValue: number;
  outPolicy: string;
  outDir?: string | null;
}

export interface BatchProgress {
  jobId: number;
  done: number;
  failed: number;
  total: number;
  current: string;
  canceled: boolean;
  finished: boolean;
}

export interface ExifInfo {
  make: string | null;
  model: string | null;
  lens: string | null;
  focalLength: string | null;
  focalLength35mm: string | null;
  fNumber: string | null;
  exposure: string | null;
  iso: number | null;
  datetime: string | null;
  gps: string | null;
  orientation: number | null;
  software: string | null;
}

/** Windows 下自定义协议经 http://<scheme>.localhost 访问（wry 行为） */
const THUMB_BASE = "http://thumb.localhost/pixlens";
const IMAGE_BASE = "http://image.localhost/pixlens";

/** v=mtime 用于击穿浏览器缓存：文件修改后同一 URL 也能刷新 */
export function thumbUrl(e: Entry, w: number): string {
  return `${THUMB_BASE}?src=${encodeURIComponent(e.path)}&w=${w}&v=${e.mtime}`;
}

/** 查看器整帧：maxDim=0 全尺寸，>0 为降采样预览；page 多页 TIFF 页码；e 曝光系数 */
export function imageUrl(e: Entry, maxDim: number, page = 0, exposure = 1): string {
  let q = `src=${encodeURIComponent(e.path)}&maxDim=${maxDim}&v=${e.mtime}`;
  if (page > 0) q += `&page=${page}`;
  if (exposure !== 1) q += `&e=${exposure}`;
  return `${IMAGE_BASE}?${q}`;
}

export const ipc = {
  scanFolder: (path: string) => invoke<ScanResult>("scan_folder", { path }),
  getBenchConfig: () => invoke<BenchConfig | null>("get_bench_config"),
  benchClearCache: () => invoke<void>("bench_clear_cache"),
  benchDone: () => invoke<void>("bench_done"),
  logBench: (metric: string, value: number) => invoke<void>("log_bench", { metric, value }),
  renamePreview: (paths: string[], template: string, start: number) =>
    invoke<RenamePair[]>("batch_rename_preview", { paths, template, start }),
  renameApply: (pairs: [string, string][]) =>
    invoke<[number, string[]]>("batch_rename_apply", { pairs }),
  batchConvert: (paths: string[], opts: ConvertOptions) =>
    invoke<number>("batch_convert", { paths, opts }),
  batchCancel: (jobId: number) => invoke<boolean>("batch_cancel", { jobId }),
  editApply: (src: string, ops: unknown, overwrite: boolean, quality: number) =>
    invoke<string>("edit_apply", { src, ops, overwrite, quality }),
  readExif: (path: string) => invoke<ExifInfo>("read_exif", { path }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setSettings: (settings: AppSettings) => invoke<number>("set_settings", { settings }),
  getLaunchFile: () => invoke<string | null>("get_launch_file"),
  pathIsDir: (path: string) => invoke<boolean>("path_is_dir", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  rememberFolder: (path: string) => invoke<void>("remember_folder", { path }),
  markThumbHintShown: () => invoke<void>("mark_thumb_hint_shown"),
  setWallpaper: (pngB64: string) => invoke<void>("set_wallpaper", { pngB64 }),
  benchMemory: () => invoke<number>("bench_memory"),
};

export function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}
