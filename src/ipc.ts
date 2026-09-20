/** Tauri IPC 封装与 thumb:// URL 构造（doc/03-架构设计 §4） */
import { invoke } from "@tauri-apps/api/core";

export interface Entry {
  name: string;
  path: string;
  ext: string;
  size: number;
  mtime: number;
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
}

/** Windows 下自定义协议经 http://<scheme>.localhost 访问（wry 行为） */
const THUMB_BASE = "http://thumb.localhost/pixlens";

/** v=mtime 用于击穿浏览器缓存：文件修改后同一 URL 也能刷新 */
export function thumbUrl(e: Entry, w: number): string {
  return `${THUMB_BASE}?src=${encodeURIComponent(e.path)}&w=${w}&v=${e.mtime}`;
}

export const ipc = {
  scanFolder: (path: string) => invoke<ScanResult>("scan_folder", { path }),
  getBenchConfig: () => invoke<BenchConfig | null>("get_bench_config"),
  benchClearCache: () => invoke<void>("bench_clear_cache"),
  benchDone: () => invoke<void>("bench_done"),
  logBench: (metric: string, value: number) => invoke<void>("log_bench", { metric, value }),
};

export function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}
