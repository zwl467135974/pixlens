/** 入口：装配 UI 事件、fs-changed 增量刷新、验收模式（doc/05 M1） */
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { ipc, type AppSettings, type Entry, type FsChanged } from "./ipc";
import { AppState, type SortDir, type SortKey } from "./state";
import { Grid } from "./grid/grid";
import { Viewer } from "./viewer/viewer";
import { CompareView } from "./compare/compare";
import { BatchPanel } from "./batch/panel";
import { EditorPanel } from "./editor/panel";
import { SettingsPanel } from "./ui/settings-panel";
import { runBench } from "./bench";

const state = new AppState();
const grid = new Grid(document.getElementById("grid") as HTMLElement);
const viewer = new Viewer(document.getElementById("app") as HTMLElement, () => state.view);
const compareView = new CompareView(document.getElementById("app") as HTMLElement);
const editorPanel = new EditorPanel(document.getElementById("app") as HTMLElement, viewer);
viewer.onEditRequest = () => editorPanel.open();

/** 多选状态（Ctrl/Shift/拖选；双击进查看器） */
const selected = new Set<string>();
let anchorPath: string | null = null;

const batchPanel = new BatchPanel(
  document.getElementById("app") as HTMLElement,
  () => imageSelection(),
  () => {
    els.btnBatch.disabled = imageSelection().length === 0;
    els.btnCompare.disabled = imageSelection().length !== 2;
  },
);

/** 双图对比：按网格顺序取选中的两张（视频不参与） */
function openCompare(): void {
  if (selected.size !== 2 || viewer.isOpen || compareView.isOpen) return;
  const picked = state.view.filter((e) => selected.has(e.path) && e.kind !== "video");
  if (picked.length === 2) compareView.open(picked[0], picked[1]);
}

/** 选中集合中的图片条目（批量转换/对比入口用，视频静默排除） */
function imageSelection(): string[] {
  return state.view.filter((e) => selected.has(e.path) && e.kind !== "video").map((e) => e.path);
}

grid.onTileOpen = (e) => {
  if (!selected.has(e.path)) {
    selected.clear();
    selected.add(e.path);
    refreshSelection();
  }
  if (e.kind === "video") {
    void ipc.openPath(e.path); // 视频交给系统默认播放器
    return;
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
  els.btnBatch.disabled = imageSelection().length === 0;
  els.btnCompare.disabled = imageSelection().length !== 2;
  status();
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const els = {
  grid: $("grid"),
  empty: $("empty"),
  btnOpen: $("btn-open"),
  btnOpen2: $("btn-open-2"),
  btnBatch: $("btn-batch") as HTMLButtonElement,
  btnCompare: $("btn-compare") as HTMLButtonElement,
  search: $("search") as HTMLInputElement,
  sortKey: $("sort-key") as HTMLSelectElement,
  sortDir: $("sort-dir"),
  size: $("size") as HTMLInputElement,
  sizeLabel: $("size-label"),
  btnSettings: $("btn-settings"),
  btnAdd: $("btn-add"),
  chips: $("folder-chips"),
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
  // 浏览密度个人习惯（滑块/Ctrl+滚轮改的）
  if (s.tileSize && s.tileSize !== Number(els.size.value)) {
    els.size.value = String(s.tileSize);
    els.sizeLabel.textContent = `${s.tileSize}px`;
    grid.setTileSize(s.tileSize);
  }
}

// ── 缩略图缩放（Ctrl+滚轮 / Ctrl+=-0 / 滑块，三者同一路径）──
/** 应用新尺寸；anchorY 为光标距网格视口顶的像素（滚轮锚定：光标下内容不跳） */
function applyTileSize(next: number, anchorY?: number): void {
  const prev = Number(els.size.value);
  els.size.value = String(next);
  els.sizeLabel.textContent = `${next}px`;
  if (anchorY !== undefined && prev !== next) {
    // 光标处内容偏移按尺寸比例换算，重排后恢复视口位置
    const offset = els.grid.scrollTop + anchorY;
    grid.setTileSize(next);
    els.grid.scrollTop = Math.max(0, (offset * next) / prev - anchorY);
  } else {
    grid.setTileSize(next);
  }
  status();
  void ipc.rememberTileSize(next);
}

let zoomTimer = 0;
/** Ctrl+滚轮：与滑块同粒度（8px/格）同边界；去抖 ~120ms 防连发缩略图请求风暴 */
function zoomStep(delta: number, anchorY?: number): void {
  const cur = Number(els.size.value);
  const next = Math.min(512, Math.max(96, cur + delta));
  if (next === cur) return;
  els.size.value = String(next);
  els.sizeLabel.textContent = `${next}px`;
  clearTimeout(zoomTimer);
  zoomTimer = window.setTimeout(() => applyTileSize(next, anchorY), 120);
}

let folders: string[] = []; // 工作集（合并浏览的文件夹列表）
let currentFolder = ""; // folders[0]（兼容既有引用）
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
  folders = [dir];
  currentFolder = dir;
  await scanInto(dir, false);
  afterWorksetChange(true);
}

/** 添加文件夹（合并浏览）：追加进当前网格，跨文件夹对比/翻页直接可用 */
async function addFolder(path?: string): Promise<void> {
  let dir: string | null | string[] = path ?? null;
  if (!dir) {
    dir = await openFileDialog({ directory: true, multiple: false });
  }
  if (typeof dir !== "string") return;
  const norm = dir.replace(/[\\/]+$/, "");
  if (folders.some((f) => f.toLowerCase() === norm.toLowerCase())) {
    status("该文件夹已在列表中");
    return;
  }
  const first = !folders.length;
  await scanInto(norm, !first);
  folders.push(norm);
  if (!currentFolder) currentFolder = norm;
  afterWorksetChange(first);
}

function removeFolder(dir: string): void {
  folders = folders.filter((f) => f !== dir);
  if (currentFolder === dir) currentFolder = folders[0] ?? "";
  const gone = state.all.filter((e) => underDir(e.path, dir)).map((e) => e.path);
  state.patch({ created: [], removed: gone, updated: [] });
  for (const p of gone) selected.delete(p);
  refreshSelection();
  if (!folders.length) {
    els.empty.classList.remove("hidden");
    els.grid.classList.add("hidden");
  }
  afterWorksetChange(false);
}

/** 工作集排序记忆键（多文件夹用并集键） */
function worksetKey(): string {
  return folders.length > 1 ? folders.join("|") : folders[0] ?? "";
}

/** path 是否位于 dir 之下（含大小写容差，Windows 不区分大小写） */
function underDir(p: string, dir: string): boolean {
  const norm = p.toLowerCase().replace(/[^\\/]+$/, "");
  return norm === dir.toLowerCase().replace(/[\\/]+$/, "") + "\\" || norm.startsWith(dir.toLowerCase().replace(/[\\/]+$/, "") + "\\");
}

/** 扫描一个目录并入网格：replace=替换全部，append=追加（多文件夹合并浏览） */
async function scanInto(dir: string, append: boolean): Promise<void> {
  currentFolder = currentFolder || dir;
  showGrid();
  status(append ? "添加文件夹…" : "扫描中…");
  const res = await ipc.scanFolder(dir);
  lastScanMs = res.elapsedMs;
  if (append) state.patch({ created: res.entries, removed: [], updated: [] });
  else state.setAll(res.entries);
  // 工作集排序记忆（有记忆才用，否则保持当前默认）
  try {
    const mem = await ipc.getFolderSort(worksetKey());
    if (mem && (mem.key !== state.sortKey || mem.dir !== state.sortDir)) {
      state.setSort(mem.key, mem.dir);
      els.sortKey.value = mem.key;
      els.sortDir.textContent = mem.dir === 1 ? "↓" : "↑";
    }
  } catch {
    /* 读取失败不阻断浏览 */
  }
  void ensureMediaDates();
}

/** 工作集变化后的公共收尾：watch 同步 / 持久化 / chips / 网格刷新 */
function afterWorksetChange(resetScroll: boolean): void {
  void ipc.setWatchFolders(folders);
  void ipc.rememberWorkset(folders);
  renderChips();
  grid.setTileSize(Number(els.size.value));
  grid.setEntries(state.view);
  if (resetScroll) els.grid.scrollTop = 0;
  status();
}

/** 文件夹来源 chips（合并浏览时的来源管理与移除入口） */
function renderChips(): void {
  const bar = els.chips;
  bar.innerHTML = "";
  if (!folders.length) {
    bar.classList.add("hidden");
    return;
  }
  bar.classList.remove("hidden");
  for (const dir of folders) {
    const chip = document.createElement("span");
    chip.className = "chip";
    const name = dir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? dir;
    const label = document.createElement("span");
    label.className = "chip-name";
    label.textContent = name;
    label.title = dir;
    const btn = document.createElement("button");
    btn.textContent = "✕";
    btn.title = "移除该文件夹";
    btn.addEventListener("click", () => removeFolder(dir));
    chip.append(label, btn);
    bar.appendChild(chip);
  }
}

let mediaDatesLoading = false;
/** "修改时间"排序用 Explorer 同款语义（内容元数据日期优先）——按文件夹惰性补齐 */
async function ensureMediaDates(): Promise<void> {
  if (mediaDatesLoading || state.sortKey !== "mtime") return;
  const todo = state.all.filter((e) => !state.mediaDates.has(e.path)).map((e) => e.path);
  if (!todo.length) return;
  mediaDatesLoading = true;
  try {
    // 分批拉取，到位即渐进重排（无感微调顺序）
    for (let i = 0; i < todo.length; i += 400) {
      const batch = todo.slice(i, i + 400);
      const items = await ipc.getMediaDates(batch);
      state.setMediaDates(
        items.map(({ path, iso }) => ({
          path,
          // EXIF 无时区按本地解析；mp4 带 Z 按 UTC
          ms: Date.parse(iso),
        })),
      );
      if (currentFolder && state.sortKey === "mtime") applyView();
    }
  } catch {
    /* 读取失败维持 mtime 兜底 */
  } finally {
    mediaDatesLoading = false;
  }
}

function applyView(keepScroll = true): void {
  const top = els.grid.scrollTop;
  grid.setEntries(state.view);
  if (keepScroll) els.grid.scrollTop = top;
  status();
}

/** 拖拽打开：文件夹 → 直接浏览；图片/视频 → 打开所在文件夹并进查看器（与双击一致） */
const MEDIA_EXTS = new Set([
  "jpg", "jpeg", "png", "gif", "webp", "bmp", "ico", "avif", "svg", "tif", "tiff", "psd", "psb", "hdr",
  "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv",
]);

async function openDropped(paths: string[]): Promise<void> {
  const first = paths.find((p) => p);
  if (!first) return;
  let dir: string;
  let viewFile: string | null = null;
  if (await ipc.pathIsDir(first)) {
    dir = first;
  } else {
    const ext = first.slice(first.lastIndexOf(".") + 1).toLowerCase();
    if (!MEDIA_EXTS.has(ext)) return; // 非媒体文件：不响应，避免扫出空网格
    dir = first.replace(/[\\/][^\\/]+$/, "");
    viewFile = first;
  }
  await openFolder(dir);
  if (viewFile && state.view.some((e) => e.path === viewFile)) {
    const entry = state.view.find((e) => e.path === viewFile)!;
    if (entry.kind === "video") void ipc.openPath(entry.path);
    else viewer.open(viewFile);
  }
}

function status(extra = ""): void {
  const folder = currentFolder || "";
  const shown = state.view.length;
  const total = state.all.length;
  // 多文件夹工作集：显示数量 + 文件夹数
  const loc = folder
    ? folders.length > 1
      ? `${total} 个文件 · ${folders.length} 个文件夹`
      : `${shown === total ? total : `${shown} / ${total}`} 个文件 · ${folder}`
    : "";
  els.statusLeft.textContent = extra ? extra : loc;
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
  els.btnCompare.addEventListener("click", openCompare);
  els.btnSettings.addEventListener("click", () => void settingsPanel.open());
  els.btnAdd.addEventListener("click", () => void addFolder());
  document.addEventListener("keydown", (ev) => {
    if (ev.ctrlKey && ev.key.toLowerCase() === "o") {
      ev.preventDefault();
      void openFolder();
    }
    // Ctrl+=/-/0：网格缩略图缩放（查看器有自己的缩放体系，网格态才响应）
    if (
      ev.ctrlKey && (ev.key === "=" || ev.key === "+" || ev.key === "-" || ev.key === "0") &&
      currentFolder && !viewer.isOpen
    ) {
      ev.preventDefault();
      if (ev.key === "0") applyTileSize(160);
      else zoomStep(ev.key === "-" ? -16 : 16);
    }
    // Ctrl+Shift+O：添加文件夹（合并浏览）
    if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "o") {
      ev.preventDefault();
      void addFolder();
    }
    // Ctrl+A 全选（查看器关闭、批量面板关闭时）
    if (ev.ctrlKey && ev.key.toLowerCase() === "a" && currentFolder && !viewer.isOpen) {
      ev.preventDefault();
      selected.clear();
      for (const e of state.view) selected.add(e.path);
      refreshSelection();
    }
    // 空格快速预览：网格态打开查看器（悬停项 > 选中项 > 首项）；输入框中不响应
    const tag = (ev.target as HTMLElement | null)?.tagName;
    if (
      ev.key === " " && currentFolder && !viewer.isOpen &&
      tag !== "INPUT" && tag !== "SELECT" && tag !== "TEXTAREA"
    ) {
      ev.preventDefault();
      const firstSel = selected.size ? [...selected][0] : null;
      const target =
        grid.hoverEntry() ??
        (firstSel ? state.view.find((e) => e.path === firstSel) : undefined) ??
        state.view[0];
      if (target) {
        if (target.kind === "video") void ipc.openPath(target.path);
        else viewer.open(target.path);
      }
    }
    // C：双图对比（恰好选中两张时）
    if (
      ev.key.toLowerCase() === "c" && !ev.ctrlKey && !ev.altKey && currentFolder &&
      !viewer.isOpen && !compareView.isOpen && selected.size === 2 &&
      tag !== "INPUT" && tag !== "SELECT" && tag !== "TEXTAREA"
    ) {
      ev.preventDefault();
      openCompare();
    }
  });

  els.sortKey.addEventListener("change", () => {
    state.setSort(els.sortKey.value as SortKey, state.sortDir);
    applyView(false);
    // 记住用户为该文件夹选的排序（与 Explorer 记忆按时间取新）
    if (worksetKey()) void ipc.rememberFolderSort(worksetKey(), state.sortKey, state.sortDir);
    if (state.sortKey === "mtime") void ensureMediaDates();
  });
  els.sortDir.addEventListener("click", () => {
    const dir: SortDir = state.sortDir === 1 ? -1 : 1;
    state.sortDir = dir;
    els.sortDir.textContent = dir === 1 ? "↓" : "↑";
    state.setSort(state.sortKey, dir);
    applyView(false);
    if (worksetKey()) void ipc.rememberFolderSort(worksetKey(), state.sortKey, dir);
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
    void ipc.rememberTileSize(Number(els.size.value));
  });
  // Ctrl+滚轮缩放（触控板捏合原生映射为 ctrlKey wheel；锚定光标下内容）
  els.grid.addEventListener(
    "wheel",
    (ev) => {
      if (!ev.ctrlKey || !currentFolder) return;
      ev.preventDefault();
      const anchorY = ev.clientY - els.grid.getBoundingClientRect().top;
      zoomStep(ev.deltaY < 0 ? 8 : -8, anchorY);
    },
    { passive: false },
  );

  // OS 级拖放（wry 拦截，HTML5 drop 不会触发）：enter/over 高亮，drop 打开
  let dragHintOn = false;
  void getCurrentWebviewWindow().onDragDropEvent((ev) => {
    const p = ev.payload;
    if (p.type === "enter" || p.type === "over") {
      if (!dragHintOn) {
        dragHintOn = true;
        document.body.classList.add("drag-over");
      }
    } else if (p.type === "leave" || p.type === "drop") {
      dragHintOn = false;
      document.body.classList.remove("drag-over");
      if (p.type === "drop") void openDropped(p.paths);
    }
  });

  void listen<FsChanged>("fs-changed", (ev) => patchFs(ev.payload));
}

/** Explorer 缓存提示（每台机器一次）：旧文件装完不重试是缓存特性，明示刷新方式 */
function showThumbHintOnce(settings: AppSettings | null): void {
  if (!settings || settings.thumbHintShown) return;
  const el = document.createElement("div");
  el.className = "thumb-hint";
  el.innerHTML =
    "<div class='thumb-hint-body'><b>资源管理器缩略图已启用</b>（PSD / PSB / 视频）。" +
    "新文件立即生效；旧文件如仍显示图标，是资源管理器缓存了旧结果，" +
    "重启资源管理器或电脑即可刷新。<button class='thumb-hint-ok'>知道了</button></div>";
  document.body.appendChild(el);
  requestAnimationFrame(() => el.classList.add("show"));
  el.querySelector(".thumb-hint-ok")?.addEventListener("click", () => {
    el.classList.remove("show");
    window.setTimeout(() => el.remove(), 400);
    void ipc.markThumbHintShown();
  });
}

async function boot(): Promise<void> {
  wireUi();
  // 启动即应用设置（主题 / 默认排序）
  let bootSettings: AppSettings | null = null;
  try {
    bootSettings = await ipc.getSettings();
    applySettings(bootSettings);
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
  // 正常模式：一次性提示 Explorer 缓存特性（横幅，不阻塞）
  showThumbHintOnce(bootSettings);
  // 双击关联文件启动：打开所在文件夹并直接进入查看器
  const launch = await ipc.getLaunchFile();
  if (launch) {
    const dir = launch.replace(/[\\/][^\\/]+$/, "");
    await openFolder(dir);
    viewer.open(launch);
    return;
  }
  // 恢复上次工作集（多文件夹合并浏览；旧版本单文件夹字段向后兼容）
  const ws: string[] = (
    bootSettings?.lastFolders?.length
      ? bootSettings.lastFolders
      : bootSettings?.lastFolder
        ? [bootSettings.lastFolder]
        : []
  ).filter((p) => typeof p === "string" && p.length > 2);
  for (const dir of ws) {
    try {
      if (!(await ipc.pathIsDir(dir))) continue;
      const first = !folders.length;
      await scanInto(dir, !first);
      folders.push(dir);
      if (!currentFolder) currentFolder = dir;
    } catch {
      /* 单个失效跳过 */
    }
  }
  if (folders.length) afterWorksetChange(true);
}

void boot();
