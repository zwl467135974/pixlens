/** 入口：装配 UI 事件、fs-changed 增量刷新、验收模式（doc/05 M1） */
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { ipc, type Entry, type FsChanged } from "./ipc";
import { AppState, type SortDir, type SortKey } from "./state";
import { Grid } from "./grid/grid";
import { runBench } from "./bench";

const state = new AppState();
const grid = new Grid(document.getElementById("grid") as HTMLElement);

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const els = {
  grid: $("grid"),
  empty: $("empty"),
  btnOpen: $("btn-open"),
  btnOpen2: $("btn-open-2"),
  search: $("search") as HTMLInputElement,
  sortKey: $("sort-key") as HTMLSelectElement,
  sortDir: $("sort-dir"),
  size: $("size") as HTMLInputElement,
  sizeLabel: $("size-label"),
  statusLeft: $("status-left"),
  statusRight: $("status-right"),
};

let currentFolder = "";
let lastScanMs = 0;
const thumbLoadCbs = new Set<(e: Entry) => void>();
grid.onThumbLoaded = (e) => thumbLoadCbs.forEach((cb) => cb(e));

function showGrid(): void {
  els.empty.classList.add("hidden");
  els.grid.classList.remove("hidden");
}

async function openFolder(path?: string): Promise<void> {
  let dir: string | null | string[] = path ?? null;
  if (!dir) {
    dir = await openFileDialog({ directory: true, multiple: false });
  }
  if (typeof dir !== "string") return;
  currentFolder = dir;
  showGrid();
  status("扫描中…");
  const res = await ipc.scanFolder(dir);
  lastScanMs = res.elapsedMs;
  state.setAll(res.entries);
  grid.setTileSize(Number(els.size.value));
  grid.setEntries(state.view);
  els.grid.scrollTop = 0;
  status();
}

function applyView(keepScroll = true): void {
  const top = els.grid.scrollTop;
  grid.setEntries(state.view);
  if (keepScroll) els.grid.scrollTop = top;
  status();
}

function status(extra = ""): void {
  const folder = currentFolder || "";
  const shown = state.view.length;
  const total = state.all.length;
  els.statusLeft.textContent = extra
    ? extra
    : folder
      ? `${shown === total ? total : `${shown} / ${total}`} 张图片 · ${folder}`
      : "";
  const parts: string[] = [];
  if (lastScanMs) parts.push(`扫描 ${lastScanMs.toFixed(0)}ms`);
  if (currentFolder && shown === 0 && state.filter)
    parts.push(`无匹配“${state.filter}”的文件`);
  parts.push(`缩略图 ${grid.tileSize}px`);
  els.statusRight.textContent = parts.join(" · ");
}

function patchFs(p: FsChanged): void {
  state.patch(p);
  applyView();
}

function wireUi(): void {
  els.btnOpen.addEventListener("click", () => void openFolder());
  els.btnOpen2.addEventListener("click", () => void openFolder());
  document.addEventListener("keydown", (ev) => {
    if (ev.ctrlKey && ev.key.toLowerCase() === "o") {
      ev.preventDefault();
      void openFolder();
    }
  });

  els.sortKey.addEventListener("change", () => {
    state.setSort(els.sortKey.value as SortKey, state.sortDir);
    applyView(false);
  });
  els.sortDir.addEventListener("click", () => {
    const dir: SortDir = state.sortDir === 1 ? -1 : 1;
    state.sortDir = dir;
    els.sortDir.textContent = dir === 1 ? "↓" : "↑";
    state.setSort(state.sortKey, dir);
    applyView(false);
  });

  let searchTimer = 0;
  els.search.addEventListener("input", () => {
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      state.setFilter(els.search.value);
      applyView(false);
    }, 120);
  });

  els.size.addEventListener("input", () => {
    els.sizeLabel.textContent = `${els.size.value}px`;
  });
  els.size.addEventListener("change", () => {
    grid.setTileSize(Number(els.size.value));
    status();
  });

  void listen<FsChanged>("fs-changed", (ev) => patchFs(ev.payload));
}

async function boot(): Promise<void> {
  wireUi();
  const bench = await ipc.getBenchConfig();
  if (bench) {
    document.title = "PixLens 图镜 · 验收模式";
    // 验收模式必须先显示网格，否则 display:none 下 clientWidth=0，虚拟滚动布局失效
    els.empty.classList.add("hidden");
    els.grid.classList.remove("hidden");
    await runBench(
      bench,
      grid,
      (entries) => {
        state.setAll(entries);
        grid.setTileSize(128);
        grid.setEntries(state.view);
      },
      thumbLoadCbs,
    );
  }
}

void boot();
