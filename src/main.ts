/** 入口：装配 UI 事件、fs-changed 增量刷新、验收模式（doc/05 M1） */
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { ipc, type AppSettings, type Entry, type FsChanged } from "./ipc";
import { AppState, type SortDir, type SortKey } from "./state";
import { Grid } from "./grid/grid";
import { Viewer } from "./viewer/viewer";
import { BatchPanel } from "./batch/panel";
import { EditorPanel } from "./editor/panel";
import { SettingsPanel } from "./ui/settings-panel";
import { runBench } from "./bench";

const state = new AppState();
const grid = new Grid(document.getElementById("grid") as HTMLElement);
const viewer = new Viewer(document.getElementById("app") as HTMLElement, () => state.view);
const editorPanel = new EditorPanel(document.getElementById("app") as HTMLElement, viewer);
viewer.onEditRequest = () => editorPanel.open();

/** 多选状态（Ctrl/Shift/拖选；双击进查看器） */
const selected = new Set<string>();
let anchorPath: string | null = null;

const batchPanel = new BatchPanel(
  document.getElementById("app") as HTMLElement,
  () => [...selected],
  () => {
    els.btnBatch.disabled = selected.size === 0;
  },
);

grid.onTileOpen = (e) => {
  if (!selected.has(e.path)) {
    selected.clear();
    selected.add(e.path);
    refreshSelection();
  }
  viewer.open(e.path);
};
grid.onTileClick = (e, ev) => {
  if (ev.ctrlKey || ev.metaKey) {
    if (selected.has(e.path)) selected.delete(e.path);
    else selected.add(e.path);
    anchorPath = e.path;
  } else if (ev.shiftKey && anchorPath) {
    const view = state.view;
    const a = view.findIndex((x) => x.path === anchorPath);
    const b = view.findIndex((x) => x.path === e.path);
    if (a >= 0 && b >= 0) {
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) selected.add(view[i].path);
    }
  } else {
    selected.clear();
    selected.add(e.path);
    anchorPath = e.path;
  }
  refreshSelection();
};

function refreshSelection(): void {
  grid.setSelected(selected);
  els.btnBatch.disabled = selected.size === 0;
  status();
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const els = {
  grid: $("grid"),
  empty: $("empty"),
  btnOpen: $("btn-open"),
  btnOpen2: $("btn-open-2"),
  btnBatch: $("btn-batch") as HTMLButtonElement,
  search: $("search") as HTMLInputElement,
  sortKey: $("sort-key") as HTMLSelectElement,
  sortDir: $("sort-dir"),
  size: $("size") as HTMLInputElement,
  sizeLabel: $("size-label"),
  btnSettings: $("btn-settings"),
  statusLeft: $("status-left"),
  statusRight: $("status-right"),
};

const settingsPanel = new SettingsPanel($("app") as HTMLElement, (s) => applySettings(s));

function applySettings(s: AppSettings): void {
  document.documentElement.dataset.theme = s.theme;
  const key = s.defaultSort as SortKey;
  if (state.sortKey !== key) {
    state.setSort(key, state.sortDir);
    els.sortKey.value = key;
    if (currentFolder) applyView(false);
  }
}

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
  if (selected.size) parts.push(`已选 ${selected.size} 张`);
  if (lastScanMs) parts.push(`扫描 ${lastScanMs.toFixed(0)}ms`);
  if (currentFolder && shown === 0 && state.filter)
    parts.push(`无匹配“${state.filter}”的文件`);
  parts.push(`缩略图 ${grid.tileSize}px`);
  els.statusRight.textContent = parts.join(" · ");
}

function patchFs(p: FsChanged): void {
  state.patch(p);
  if (p.removed.length) {
    for (const r of p.removed) selected.delete(r);
  }
  // 更新/改名后旧路径失效
  const live = new Set(state.all.map((e) => e.path));
  for (const s of [...selected]) if (!live.has(s)) selected.delete(s);
  applyView();
  refreshSelection();
}

function wireUi(): void {
  els.btnOpen.addEventListener("click", () => void openFolder());
  els.btnOpen2.addEventListener("click", () => void openFolder());
  els.btnBatch.addEventListener("click", () => batchPanel.open());
  els.btnSettings.addEventListener("click", () => void settingsPanel.open());
  document.addEventListener("keydown", (ev) => {
    if (ev.ctrlKey && ev.key.toLowerCase() === "o") {
      ev.preventDefault();
      void openFolder();
    }
    // Ctrl+A 全选（查看器关闭、批量面板关闭时）
    if (ev.ctrlKey && ev.key.toLowerCase() === "a" && currentFolder && !viewer.isOpen) {
      ev.preventDefault();
      selected.clear();
      for (const e of state.view) selected.add(e.path);
      refreshSelection();
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
  // 启动即应用设置（主题 / 默认排序）
  try {
    applySettings(await ipc.getSettings());
  } catch {
    /* 默认深色 */
  }
  const bench = await ipc.getBenchConfig();
  if (bench) {
    document.title = "PixLens 图镜 · 验收模式";
    // 验收模式必须先显示网格，否则 display:none 下 clientWidth=0，虚拟滚动布局失效
    els.empty.classList.add("hidden");
    els.grid.classList.remove("hidden");
    await runBench(
      bench,
      grid,
      viewer,
      (entries) => {
        state.setAll(entries);
        grid.setTileSize(128);
        grid.setEntries(state.view);
      },
      thumbLoadCbs,
    );
    return;
  }
  // 双击关联文件启动：打开所在文件夹并直接进入查看器
  const launch = await ipc.getLaunchFile();
  if (launch) {
    const dir = launch.replace(/[\\/][^\\/]+$/, "");
    await openFolder(dir);
    viewer.open(launch);
  }
}

void boot();
