/** Canvas 大图查看器（doc/01 F2、doc/03 §3.3）
 *
 * - 变换矩阵：translate(offset) → rotate → scale(含翻转) → 以自然尺寸绘制
 *   （缩放为均匀标量，故"以光标为中心缩放"公式 off' = q - f·(q-off) 在旋转/翻转下依然成立）
 * - LOD：预览（maxDim=2560 降采样）先行显示，缩放接近预览原生分辨率时升级全尺寸
 * - GIF：叠加层 <img> 播放动画（Canvas 不做帧动画），共用同一套变换
 * - 翻页预取：当前 ±2 张预览解码进内存 LRU（容量 8）
 */
import { fmtSize, imageUrl, ipc, type Entry } from "../ipc";

const IMAGE_BASE = "http://image.localhost/pixlens";
export const PREVIEW_MAX_DIM = 2560;
const LRU_CAPACITY = 8;
const MIN_SCALE = 0.02;
const MAX_SCALE = 40;

type Source = { kind: "bitmap"; bmp: ImageBitmap } | { kind: "img"; img: HTMLImageElement };

export interface Frame {
  src: Source;
  /** 原始（全尺寸）宽高 —— 变换基准 */
  naturalW: number;
  naturalH: number;
  /** 当前源的像素宽（LOD 升级阈值用） */
  nativeW: number;
  /** 是否已是最优源（透传格式预览=原始，无需升级） */
  isFull: boolean;
  /** 多页文件总页数（TIFF，其他为 null） */
  pages: number | null;
}

/** 曝光滑块对这些格式有意义（32 位内容 / HDR 载体） */
const EXPOSURE_EXTS = new Set(["psd", "psb", "tif", "tiff", "hdr"]);

export class Viewer {
  private root: HTMLElement;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private gifLayer: HTMLImageElement;
  private infoPanel: HTMLElement;
  private zoomLabel: HTMLElement;
  private pagesWrap: HTMLElement;
  private pageLabel: HTMLElement;
  private expWrap: HTMLElement;
  private expSlider: HTMLInputElement;
  private expVal: HTMLElement;
  private cropRectEl: HTMLElement;
  private hintEl: HTMLElement;
  private hintTimer = 0;
  /** 裁剪框选模式（编辑器用） */
  private cropMode = false;
  /** 编辑预览滤镜（ctx.filter / CSS filter 双路径共用） */
  private previewFilter = "none";
  private cropAnchor: { x: number; y: number } | null = null;
  private cropCb: ((rect: { x1: number; y1: number; x2: number; y2: number }) => void) | null = null;
  private cropDragging = false;

  private getEntries: () => Entry[];
  private idx = -1;
  private cur: Entry | null = null;
  /** 预览 LRU（按插入序淘汰，键含页码/曝光变体） */
  private lru = new Map<string, Frame>();
  private fullFrame: Frame | null = null;
  private fullToken = 0;
  /** 多页 TIFF 当前页（0 基） */
  page = 0;
  /** HDR 曝光系数（滑块 stops = log2） */
  exposure = 1;

  scale = 1;
  rot = 0;
  flipH = false;
  flipV = false;
  offX = 0;
  offY = 0;

  private open_ = false;
  private slideshowTimer = 0;
  slideshowMs = 5000;
  private infoVisible = false;

  /** 每次绘制完成回调（bench 埋点） */
  onDrawn: (e: Entry) => void = () => {};
  /** 全尺寸帧就绪回调（bench 埋点） */
  onFullLoaded: (e: Entry) => void = () => {};
  /** 编辑请求回调（工具栏"编辑"按钮 / E 键） */
  onEditRequest: () => void = () => {};

  constructor(container: HTMLElement, getEntries: () => Entry[]) {
    this.getEntries = getEntries;
    const root = document.createElement("div");
    root.id = "viewer";
    root.className = "hidden";
    root.innerHTML = `
      <canvas id="viewer-canvas"></canvas>
      <img id="viewer-gif" class="hidden" alt="" draggable="false" />
      <div id="viewer-info" class="hidden"></div>
      <div id="viewer-bar">
        <button data-act="prev" title="上一张（←）">◀</button>
        <button data-act="next" title="下一张（→）">▶</button>
        <span class="vsep"></span>
        <span id="viewer-pages" class="vpages hidden">
          <button data-act="pgprev" title="上一页（PgUp）">‹</button>
          <span id="viewer-page-label">1/1</span>
          <button data-act="pgnext" title="下一页（PgDn）">›</button>
        </span>
        <span class="vsep"></span>
        <button data-act="rotl" title="左旋 90°（L）">⟲</button>
        <button data-act="rotr" title="右旋 90°（R）">⟳</button>
        <button data-act="fliph" title="水平翻转">⇋</button>
        <button data-act="flipv" title="垂直翻转">⇵</button>
        <span class="vsep"></span>
        <button data-act="fit" title="适应窗口（0）">适应</button>
        <button data-act="100" title="100%（1）">100%</button>
        <span class="vsep"></span>
        <button data-act="wallpaper" title="设为桌面壁纸">壁纸</button>
        <label id="viewer-exposure" class="vexp hidden" title="HDR 曝光">EV
          <input id="exposure-slider" type="range" min="-2" max="2" step="0.1" value="0" />
          <span id="exposure-val">0</span>
        </label>
        <span class="vsep"></span>
        <button data-act="fs" title="全屏（F）">全屏</button>
        <button data-act="slideshow" title="幻灯片（空格）">▶ 幻灯片</button>
        <select id="slideshow-interval" title="幻灯片间隔">
          <option value="2000">2s</option>
          <option value="5000" selected>5s</option>
          <option value="10000">10s</option>
        </select>
        <span class="vsep"></span>
        <button data-act="info" title="图片信息（I）">ℹ</button>
        <button data-act="edit" title="编辑（E）">✎ 编辑</button>
        <button data-act="close" title="关闭（Esc）">✕</button>
        <span id="viewer-zoom" class="zoom"></span>
      </div>
      <div id="viewer-crop-rect" class="hidden"></div>
      <div id="viewer-hint"></div>`;
    container.appendChild(root);
    this.root = root;
    this.canvas = root.querySelector("#viewer-canvas") as HTMLCanvasElement;
    this.ctx = this.canvas.getContext("2d", { alpha: false })!;
    this.gifLayer = root.querySelector("#viewer-gif") as HTMLImageElement;
    this.infoPanel = root.querySelector("#viewer-info") as HTMLElement;
    this.zoomLabel = root.querySelector("#viewer-zoom") as HTMLElement;
    this.pagesWrap = root.querySelector("#viewer-pages") as HTMLElement;
    this.pageLabel = root.querySelector("#viewer-page-label") as HTMLElement;
    this.expWrap = root.querySelector("#viewer-exposure") as HTMLElement;
    this.expSlider = root.querySelector("#exposure-slider") as HTMLInputElement;
    this.expVal = root.querySelector("#exposure-val") as HTMLElement;
    this.cropRectEl = root.querySelector("#viewer-crop-rect") as HTMLElement;
    this.hintEl = root.querySelector("#viewer-hint") as HTMLElement;

    this.bindInput();
    (root.querySelector("#viewer-bar") as HTMLElement).addEventListener("click", (ev) => {
      const act = (ev.target as HTMLElement).closest("button")?.dataset.act;
      if (act) void this.action(act);
    });
    const sel = root.querySelector("#slideshow-interval") as HTMLSelectElement;
    sel.addEventListener("change", () => {
      this.slideshowMs = Number(sel.value);
    });
    // 曝光滑块：防抖 150ms 后重解码（预览+全尺寸）
    let expTimer = 0;
    this.expSlider.addEventListener("input", () => {
      this.expVal.textContent = this.expSlider.value;
      window.clearTimeout(expTimer);
      expTimer = window.setTimeout(() => this.setExposure(Number(this.expSlider.value)), 150);
    });
  }

  get isOpen(): boolean {
    return this.open_;
  }

  /** 当前条目（编辑器保存用） */
  currentEntry(): Entry | null {
    return this.cur;
  }

  /** 屏幕坐标 → 原图像素坐标（逆变换：off → 缩放 → 反旋转，绕图像中心） */
  screenToImage(cx: number, cy: number): { x: number; y: number } | null {
    const f = this.effective();
    if (!f) return null;
    const s = this.scale;
    const sx = this.flipH ? -s : s;
    const sy = this.flipV ? -s : s;
    const rot = (-this.rot * Math.PI) / 180;
    const dx = (cx - this.offX) / sx;
    const dy = (cy - this.offY) / sy;
    const cos = Math.cos(rot);
    const sin = Math.sin(rot);
    return {
      x: dx * cos - dy * sin + f.naturalW / 2,
      y: dx * sin + dy * cos + f.naturalH / 2,
    };
  }

  /** 编辑预览：ctx.filter（Canvas 绘制路径）+ CSS filter（GIF 叠加层），与 Rust 应用公式一致 */
  setPreviewFilter(css: string): void {
    this.previewFilter = css;
    this.gifLayer.style.filter = css === "none" ? "" : css;
    this.draw();
  }

  /** 编辑保存后刷新当前图（清 LRU 与全尺寸帧后重载，mtime 变化使缓存键更新） */
  reloadCurrent(): void {
    if (!this.cur) return;
    const path = this.cur.path;
    for (const k of [...this.lru.keys()]) {
      if (k === path || k.startsWith(`${path}#`)) this.lru.delete(k);
    }
    this.fullFrame = null;
    void this.show(this.idx);
  }

  /** 进入裁剪框选模式：拖拽出矩形后回调（屏幕坐标） */
  enterCropMode(cb: (rect: { x1: number; y1: number; x2: number; y2: number }) => void): void {
    this.cropMode = true;
    this.cropCb = cb;
    this.canvas.style.cursor = "crosshair";
    this.cropRectEl.classList.add("hidden");
  }

  exitCropMode(): void {
    this.cropMode = false;
    this.cropCb = null;
    this.cropAnchor = null;
    this.canvas.style.cursor = "grab";
    this.cropRectEl.classList.add("hidden");
  }

  open(path: string): void {
    const entries = this.getEntries();
    const i = entries.findIndex((e) => e.path === path);
    if (i < 0) return;
    this.open_ = true;
    this.root.classList.remove("hidden");
    // 入场过渡（重启动画：先移除再强制回流再加回）
    this.root.classList.remove("open");
    void this.root.offsetWidth;
    this.root.classList.add("open");
    this.resize();
    void this.show(i);
  }

  close(): void {
    this.open_ = false;
    this.stopSlideshow();
    if (document.fullscreenElement) void document.exitFullscreen();
    // 退场过渡：先淡出，250ms 后真正隐藏
    this.root.classList.remove("open");
    window.setTimeout(() => {
      this.root.classList.add("hidden");
    }, 240);
    this.cur = null;
    this.fullFrame = null;
    this.gifLayer.src = "";
  }

  /** 下一张（dir=±1，循环） */
  next(dir: 1 | -1): void {
    if (!this.open_) return;
    const entries = this.getEntries();
    if (!entries.length) return;
    const i = (this.idx + dir + entries.length) % entries.length;
    void this.show(i);
  }

  /** bench：预览下一张路径 */
  peekNext(dir: 1 | -1): Entry | null {
    const entries = this.getEntries();
    if (!entries.length) return null;
    return entries[(this.idx + dir + entries.length) % entries.length] ?? null;
  }

  /** bench：当前条目 */
  current(): Entry | null {
    return this.cur;
  }

  /** bench：跳转（不预取场景） */
  jump(n: number): void {
    const entries = this.getEntries();
    if (!entries.length) return;
    void this.show((this.idx + n + entries.length) % entries.length);
  }

  private async show(i: number): Promise<void> {
    const entries = this.getEntries();
    const entry = entries[i];
    if (!entry) return;
    const token = ++this.fullToken;
    this.idx = i;
    this.cur = entry;
    this.fullFrame = null;
    this.rot = 0;
    this.flipH = false;
    this.flipV = false;
    this.page = 0; // 换文件重置到第一页
    this.gifLayer.classList.add("hidden");
    this.updateVariantUi();

    await this.getPreview(entry, this.page, this.exposure);
    if (token !== this.fullToken || this.cur?.path !== entry.path) return;
    // 默认按原始大小显示（100%，不放大铺满）；适应窗口用 0 键/按钮/双击切换
    this.pageFade = 0;
    this.runPageFade();
    this.animT = null; // 直接设目标（新图从 100% 起步，不播旧变换的插值）
    this.scale = 1;
    this.offX = this.canvas.clientWidth / 2;
    this.offY = this.canvas.clientHeight / 2;
    this.draw();
    this.updateVariantUi();
    this.prefetch(i);
    void this.loadFull(entry, token);
    this.loadExif(entry);
  }

  /** 拍摄信息（EXIF）：懒加载，到达后若信息面板开着则刷新 */
  private curExif: { path: string; info: import("../ipc").ExifInfo | null } | null = null;

  private loadExif(entry: Entry): void {
    // 仅 JPG/TIFF 有 EXIF（容品限制），其他格式直接置空
    if (!["jpg", "jpeg", "tif", "tiff"].includes(entry.ext)) {
      this.curExif = { path: entry.path, info: null };
      return;
    }
    this.curExif = null;
    void ipc
      .readExif(entry.path)
      .then((info) => {
        if (this.cur?.path !== entry.path) return;
        this.curExif = { path: entry.path, info };
        if (this.infoVisible) this.draw();
      })
      .catch(() => {
        if (this.cur?.path === entry.path) this.curExif = { path: entry.path, info: null };
      });
  }

  /** 翻页/打开时图片 180ms 淡入 */
  private pageFade = 1;
  private fadeActive = false;

  private runPageFade(): void {
    if (this.fadeActive) return;
    this.fadeActive = true;
    const t0 = performance.now();
    const step = () => {
      this.pageFade = Math.min(1, (performance.now() - t0) / 180);
      this.draw();
      if (this.pageFade < 1 && this.open_) {
        requestAnimationFrame(step);
      } else {
        this.fadeActive = false;
      }
    };
    requestAnimationFrame(step);
  }

  /** LRU 键：路径 + 页码/曝光变体（仅相关格式带变体后缀） */
  private vkey(e: Entry, page: number, exposure: number): string {
    let k = e.path;
    if (e.ext === "tif" || e.ext === "tiff") k += `#p${page}`;
    if (EXPOSURE_EXTS.has(e.ext)) k += `#e${Math.round(exposure * 100)}`;
    return k;
  }

  private async getPreview(entry: Entry, page: number, exposure: number): Promise<Frame> {
    const key = this.vkey(entry, page, exposure);
    const hit = this.lru.get(key);
    if (hit) {
      // LRU 置新
      this.lru.delete(key);
      this.lru.set(key, hit);
      return hit;
    }
    const frame = await loadFrame(entry, PREVIEW_MAX_DIM, page, exposure);
    this.lru.set(key, frame);
    while (this.lru.size > LRU_CAPACITY) {
      const oldest = this.lru.keys().next().value as string;
      if (oldest === key) break;
      this.lru.delete(oldest);
    }
    return frame;
  }

  private prefetch(i: number): void {
    const entries = this.getEntries();
    for (const d of [-2, -1, 1, 2]) {
      const e = entries[i + d];
      if (e && !this.lru.has(this.vkey(e, 0, 1))) {
        void this.getPreview(e, 0, 1).catch(() => {});
      }
    }
  }

  /** LOD 升级：全尺寸替换（透传格式浏览器原生解码） */
  private async loadFull(entry: Entry, token: number): Promise<void> {
    const cur = await this.getPreview(entry, this.page, this.exposure); // 确保已在 LRU
    if (cur.isFull) return; // 透传格式预览即全尺寸
    const frame = await loadFrame(entry, 0, this.page, this.exposure);
    if (token !== this.fullToken || this.cur?.path !== entry.path) return;
    this.fullFrame = frame;
    this.draw();
    this.updateVariantUi();
    this.onFullLoaded(entry);
  }

  private effective(): Frame | null {
    if (!this.cur) return null;
    return (
      this.fullFrame ?? this.lru.get(this.vkey(this.cur, this.page, this.exposure)) ?? null
    );
  }

  /** 多页 TIFF 翻页（dir=±1）；页码越界由 Rust 返回错误 → 占位 */
  setPage(dir: 1 | -1): void {
    if (!this.open_ || !this.cur) return;
    const target = this.page + dir;
    if (target < 0) return;
    const curPages = this.effective()?.pages ?? 1;
    if (target >= curPages) return;
    this.page = target;
    this.fullFrame = null;
    const token = ++this.fullToken;
    const entry = this.cur;
    void (async () => {
      await this.getPreview(entry, this.page, this.exposure);
      if (token !== this.fullToken || this.cur?.path !== entry.path) return;
      this.draw();
      this.updateVariantUi();
      void this.loadFull(entry, token);
      // 预取下一页（多页翻页流畅性）
      const pages = this.effective()?.pages ?? 0;
      if (pages > this.page + 1) {
        void this.getPreview(entry, this.page + 1, this.exposure).catch(() => {});
      }
    })();
  }

  /** HDR 曝光（stops，滑块） */
  setExposure(stops: number): void {
    if (!this.open_ || !this.cur) return;
    this.exposure = Math.pow(2, stops);
    this.fullFrame = null;
    const token = ++this.fullToken;
    const entry = this.cur;
    void (async () => {
      await this.getPreview(entry, this.page, this.exposure);
      if (token !== this.fullToken || this.cur?.path !== entry.path) return;
      this.draw();
      void this.loadFull(entry, token);
    })();
  }

  /** 页码/曝光控件可见性与数值刷新 */
  private updateVariantUi(): void {
    const f = this.effective();
    const pages = f?.pages ?? (this.cur && (this.cur.ext === "tif" || this.cur.ext === "tiff") ? 1 : 0);
    if (pages > 1) {
      this.pagesWrap.classList.remove("hidden");
      this.pageLabel.textContent = `${this.page + 1}/${pages}`;
    } else {
      this.pagesWrap.classList.add("hidden");
    }
    if (this.cur && EXPOSURE_EXTS.has(this.cur.ext)) {
      this.expWrap.classList.remove("hidden");
      this.expVal.textContent = `${Math.round(Math.log2(this.exposure) * 10) / 10}`;
    } else {
      this.expWrap.classList.add("hidden");
    }
  }

  // ── 变换（缩放/平移/旋转均带 160ms 平滑插值；bench 的 panBy 保持直设） ──
  private fit(): void {
    const f = this.effective();
    if (!f) return;
    const rotated = this.rot % 180 !== 0;
    const w = rotated ? f.naturalH : f.naturalW;
    const h = rotated ? f.naturalW : f.naturalH;
    const cw = this.canvas.clientWidth;
    const ch = this.canvas.clientHeight;
    this.animateTo(Math.min(cw / w, ch / h), cw / 2, ch / 2);
  }

  private one(): void {
    this.animateTo(1, this.canvas.clientWidth / 2, this.canvas.clientHeight / 2);
  }

  zoomAt(cx: number, cy: number, factor: number): void {
    const ns = Math.min(MAX_SCALE, Math.max(MIN_SCALE, this.scale * factor));
    const f = ns / this.scale;
    this.animateTo(ns, cx - f * (cx - this.offX), cy - f * (cy - this.offY));
  }

  panBy(dx: number, dy: number): void {
    this.offX += dx;
    this.offY += dy;
    this.draw();
  }

  rotate(deg: number): void {
    const target = (this.rot + deg + 360) % 360;
    // 最短弧插值（90° 步进天然最短）
    this.animateTo(this.scale, this.canvas.clientWidth / 2, this.canvas.clientHeight / 2, target);
  }

  flip(axis: "h" | "v"): void {
    if (axis === "h") this.flipH = !this.flipH;
    else this.flipV = !this.flipV;
    this.draw();
  }

  // ── 变换动画（指数插值，收敛即停） ─────────
  private animActive = false;
  private animT: { s: number; x: number; y: number; r: number } | null = null;

  private animateTo(s: number, x: number, y: number, r?: number): void {
    this.animT = { s, x, y, r: r ?? this.rot };
    if (!this.animActive) {
      this.animActive = true;
      requestAnimationFrame(() => this.animStep());
    }
  }

  private animStep(): void {
    const t = this.animT;
    if (!t || !this.open_) {
      this.animActive = false;
      return;
    }
    const k = 0.3; // 插值系数（每帧靠近目标 30%）
    this.scale += (t.s - this.scale) * k;
    this.offX += (t.x - this.offX) * k;
    this.offY += (t.y - this.offY) * k;
    // 旋转最短弧
    let dr = t.r - this.rot;
    if (dr > 180) dr -= 360;
    if (dr < -180) dr += 360;
    this.rot = (this.rot + dr * k + 360) % 360;
    this.draw();
    const done =
      Math.abs(t.s - this.scale) < 1e-4 &&
      Math.abs(t.x - this.offX) < 0.3 &&
      Math.abs(t.y - this.offY) < 0.3 &&
      Math.abs(dr) < 0.05;
    if (done) {
      this.scale = t.s;
      this.offX = t.x;
      this.offY = t.y;
      this.rot = t.r;
      this.animT = null;
      this.animActive = false;
      this.draw();
    } else {
      requestAnimationFrame(() => this.animStep());
    }
  }

  // ── 绘制 ─────────────────────────────
  private resize(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = this.root.clientWidth;
    const h = this.root.clientHeight;
    this.canvas.width = Math.round(w * dpr);
    this.canvas.height = Math.round(h * dpr);
    this.canvas.style.width = `${w}px`;
    this.canvas.style.height = `${h}px`;
  }

  /** 重绘（编辑器同步视图用） */
  draw(): void {
    const entry = this.cur;
    const f = this.effective();
    const dpr = window.devicePixelRatio || 1;
    const ctx = this.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.filter = "none";
    ctx.fillStyle = "#111116";
    ctx.fillRect(0, 0, this.canvas.clientWidth, this.canvas.clientHeight);
    if (!f || !entry) return;

    if (f.src.kind === "img") {
      // GIF/AVIF/SVG：叠加层 <img>（GIF 动画），Canvas 只铺底色
      const img = f.src.img;
      if (this.gifLayer.src !== img.src) {
        this.gifLayer.src = img.src;
      }
      this.gifLayer.classList.remove("hidden");
      this.gifLayer.style.width = `${f.naturalW}px`;
      this.gifLayer.style.height = `${f.naturalH}px`;
      this.gifLayer.style.transform = this.cssTransform();
      this.gifLayer.style.opacity = `${this.pageFade}`;
      this.zoomLabel.textContent = `${Math.round(this.scale * 100)}%`;
    } else {
      this.gifLayer.classList.add("hidden");
      // 编辑预览滤镜：只在绘制图像时启用（底色不受影响）
      ctx.filter = this.previewFilter;
      ctx.globalAlpha = this.pageFade;
      ctx.translate(this.offX, this.offY);
      ctx.rotate((this.rot * Math.PI) / 180);
      const sx = this.flipH ? -this.scale : this.scale;
      const sy = this.flipV ? -this.scale : this.scale;
      ctx.scale(sx, sy);
      // >100% 时关闭平滑（像素视图），<100% 时开启（缩小时抗锯齿）
      ctx.imageSmoothingEnabled = this.scale < 1;
      ctx.drawImage(f.src.bmp, -f.naturalW / 2, -f.naturalH / 2, f.naturalW, f.naturalH);
      ctx.globalAlpha = 1;
      ctx.filter = "none";
      this.zoomLabel.textContent = `${Math.round(this.scale * 100)}%`;
    }
    if (this.infoVisible) this.renderInfo(entry, f);
    this.onDrawn(entry);
  }

  private cssTransform(): string {
    const sx = this.flipH ? -this.scale : this.scale;
    const sy = this.flipV ? -this.scale : this.scale;
    return `translate(${this.offX}px, ${this.offY}px) rotate(${this.rot}deg) scale(${sx}, ${sy}) translate(-50%, -50%)`;
  }

  private renderInfo(e: Entry, f: Frame): void {
    this.infoPanel.classList.remove("hidden");
    const pageInfo = f.pages && f.pages > 1 ? ` · 第 ${this.page + 1}/${f.pages} 页` : "";
    const exif = this.curExif?.path === e.path ? this.curExif.info : null;
    const photoRows: string[] = [];
    if (exif) {
      const camera = [exif.make, exif.model].filter(Boolean).join(" ");
      if (camera) photoRows.push(camera);
      if (exif.lens) photoRows.push(exif.lens);
      const params = [
        exif.focalLength,
        exif.focalLength35mm,
        exif.fNumber,
        exif.exposure,
        exif.iso ? `ISO ${exif.iso}` : null,
      ]
        .filter(Boolean)
        .join(" · ");
      if (params) photoRows.push(params);
      if (exif.datetime) photoRows.push(exif.datetime);
      if (exif.gps) photoRows.push(`📍 ${exif.gps}`);
    }
    const exifBlock = photoRows.length
      ? `<div class="exif-sep"></div>${photoRows.map((r) => `<div class="row">${r}</div>`).join("")}`
      : "";
    this.infoPanel.innerHTML = `
      <div class="row name">${e.name}</div>
      <div class="row">${f.naturalW} × ${f.naturalH} px · ${e.ext.toUpperCase()}${pageInfo}</div>
      <div class="row">${fmtSize(e.size)} · 缩放 ${Math.round(this.scale * 100)}%</div>
      ${exifBlock}
      <div class="row dim">${e.path}</div>`;
  }

  // ── 交互 ─────────────────────────────
  private bindInput(): void {
    this.canvas.addEventListener("wheel", (ev) => {
      ev.preventDefault();
      const rect = this.canvas.getBoundingClientRect();
      this.zoomAt(ev.clientX - rect.left, ev.clientY - rect.top, ev.deltaY < 0 ? 1.2 : 1 / 1.2);
    });

    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    this.root.addEventListener("mousedown", (ev) => {
      if (this.cropMode) {
        // 裁剪框选（相对视口的画布坐标）
        const rect = this.canvas.getBoundingClientRect();
        this.cropAnchor = { x: ev.clientX - rect.left, y: ev.clientY - rect.top };
        this.cropDragging = true;
        this.updateCropOverlay(this.cropAnchor, this.cropAnchor);
        this.cropRectEl.classList.remove("hidden");
        return;
      }
      dragging = true;
      lastX = ev.clientX;
      lastY = ev.clientY;
    });
    window.addEventListener("mousemove", (ev) => {
      if (this.cropDragging && this.cropAnchor) {
        const rect = this.canvas.getBoundingClientRect();
        const cur = {
          x: Math.max(0, Math.min(rect.width, ev.clientX - rect.left)),
          y: Math.max(0, Math.min(rect.height, ev.clientY - rect.top)),
        };
        this.updateCropOverlay(this.cropAnchor, cur);
        return;
      }
      if (!dragging || !this.open_) return;
      this.panBy(ev.clientX - lastX, ev.clientY - lastY);
      lastX = ev.clientX;
      lastY = ev.clientY;
    });
    window.addEventListener("mouseup", (ev) => {
      if (this.cropDragging && this.cropAnchor) {
        this.cropDragging = false;
        const rect = this.canvas.getBoundingClientRect();
        const cur = {
          x: Math.max(0, Math.min(rect.width, ev.clientX - rect.left)),
          y: Math.max(0, Math.min(rect.height, ev.clientY - rect.top)),
        };
        const a = this.cropAnchor;
        this.cropAnchor = null;
        // 太小的框视为误触
        if (Math.abs(cur.x - a.x) > 8 && Math.abs(cur.y - a.y) > 8 && this.cropCb) {
          this.cropCb({
            x1: Math.min(a.x, cur.x),
            y1: Math.min(a.y, cur.y),
            x2: Math.max(a.x, cur.x),
            y2: Math.max(a.y, cur.y),
          });
        } else {
          this.cropRectEl.classList.add("hidden");
        }
        return;
      }
      dragging = false;
    });

    this.canvas.addEventListener("dblclick", () => {
      if (Math.abs(this.scale - 1) < 0.01) this.fit();
      else this.one();
      this.draw();
    });

    window.addEventListener("resize", () => {
      if (!this.open_) return;
      this.resize();
      this.draw();
    });

    // 鼠标侧键（前进/后退键）翻页
    window.addEventListener("auxclick", (ev) => {
      if (!this.open_) return;
      if (ev.button === 3) {
        ev.preventDefault();
        this.next(-1);
      } else if (ev.button === 4) {
        ev.preventDefault();
        this.next(1);
      }
    });

    window.addEventListener("keydown", (ev) => {
      if (!this.open_) return;
      const k = ev.key;
      if (k === "ArrowLeft") {
        ev.preventDefault();
        this.next(-1);
      } else if (k === "ArrowRight") {
        ev.preventDefault();
        this.next(1);
      } else if (k === "+" || k === "=") {
        this.zoomAt(this.canvas.clientWidth / 2, this.canvas.clientHeight / 2, 1.25);
      } else if (k === "-") {
        this.zoomAt(this.canvas.clientWidth / 2, this.canvas.clientHeight / 2, 1 / 1.25);
      } else if (k === "0") {
        this.fit();
        this.draw();
      } else if (k === "1") {
        this.one();
        this.draw();
      } else if (k === "r" || k === "R") {
        this.rotate(90);
      } else if (k === "l" || k === "L") {
        this.rotate(-90);
      } else if (k === "f" || k === "F") {
        void this.toggleFullscreen();
      } else if (k === " ") {
        ev.preventDefault();
        this.toggleSlideshow();
      } else if (k === "PageUp") {
        ev.preventDefault();
        this.setPage(-1);
      } else if (k === "PageDown") {
        ev.preventDefault();
        this.setPage(1);
      } else if (k === "i" || k === "I") {
        this.infoVisible = !this.infoVisible;
        if (!this.infoVisible) this.infoPanel.classList.add("hidden");
        this.draw();
      } else if (k === "e" || k === "E") {
        this.onEditRequest();
      } else if (k === "Escape") {
        if (this.cropMode) {
          this.exitCropMode();
        } else {
          this.close();
        }
      }
    });
  }

  /** 裁剪框选覆盖层 */
  private updateCropOverlay(a: { x: number; y: number }, b: { x: number; y: number }): void {
    const x = Math.min(a.x, b.x);
    const y = Math.min(a.y, b.y);
    const w = Math.abs(b.x - a.x);
    const h = Math.abs(b.y - a.y);
    const el = this.cropRectEl;
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.style.width = `${w}px`;
    el.style.height = `${h}px`;
  }

  private async action(act: string): Promise<void> {
    switch (act) {
      case "prev": this.next(-1); break;
      case "next": this.next(1); break;
      case "pgprev": this.setPage(-1); break;
      case "pgnext": this.setPage(1); break;
      case "rotl": this.rotate(-90); break;
      case "rotr": this.rotate(90); break;
      case "fliph": this.flip("h"); break;
      case "flipv": this.flip("v"); break;
      case "fit": this.fit(); this.draw(); break;
      case "100": this.one(); this.draw(); break;
      case "fs": await this.toggleFullscreen(); break;
      case "wallpaper": await this.setAsWallpaper(); break;
      case "slideshow": this.toggleSlideshow(); break;
      case "info":
        this.infoVisible = !this.infoVisible;
        if (!this.infoVisible) this.infoPanel.classList.add("hidden");
        this.draw();
        break;
      case "edit": this.onEditRequest(); break;
      case "close": this.close(); break;
    }
  }

  private async toggleFullscreen(): Promise<void> {
    if (document.fullscreenElement) {
      await document.exitFullscreen();
    } else {
      // 对整页全屏：编辑/设置等浮层面板在全屏下仍然可见
      await document.documentElement.requestFullscreen();
    }
    this.resize();
    this.draw();
  }

  /** 顶部轻提示（壁纸设置等异步操作反馈） */
  private flashHint(text: string): void {
    this.hintEl.textContent = text;
    this.hintEl.classList.add("show");
    window.clearTimeout(this.hintTimer);
    this.hintTimer = window.setTimeout(() => this.hintEl.classList.remove("show"), 1800);
  }

  /** 当前图导出为屏幕分辨率 PNG 并设为壁纸（全格式统一走位图路径，含编辑预览滤镜） */
  private async setAsWallpaper(): Promise<void> {
    const f = this.effective();
    if (!f) return;
    this.flashHint("正在导出…");
    try {
      const dpr = window.devicePixelRatio || 1;
      const maxDim = Math.max(window.screen.width, window.screen.height) * dpr;
      const scale = Math.min(1, maxDim / Math.max(f.naturalW, f.naturalH));
      const w = Math.max(1, Math.round(f.naturalW * scale));
      const h = Math.max(1, Math.round(f.naturalH * scale));
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      const cx = c.getContext("2d")!;
      cx.filter = this.previewFilter;
      cx.drawImage(f.src.kind === "bitmap" ? f.src.bmp : f.src.img, 0, 0, w, h);
      const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
      if (!blob) throw new Error("导出失败");
      const dataUrl = await new Promise<string>((r, rej) => {
        const fr = new FileReader();
        fr.onload = () => r(fr.result as string);
        fr.onerror = () => rej(fr.error);
        fr.readAsDataURL(blob);
      });
      await ipc.setWallpaper(dataUrl.slice(dataUrl.indexOf(",") + 1));
      this.flashHint("已设为桌面壁纸");
    } catch (e) {
      this.flashHint(`设置失败：${e instanceof Error ? e.message : String(e)}`);
    }
  }

  toggleSlideshow(): void {
    if (this.slideshowTimer) this.stopSlideshow();
    else this.startSlideshow();
  }

  private startSlideshow(): void {
    if (!document.fullscreenElement) {
      void document.documentElement.requestFullscreen();
    }
    this.slideshowTimer = window.setInterval(() => this.next(1), this.slideshowMs);
  }

  private stopSlideshow(): void {
    if (this.slideshowTimer) {
      window.clearInterval(this.slideshowTimer);
      this.slideshowTimer = 0;
    }
  }

  // ── bench 专用 ───────────────────────
  async benchWaitFull(entry: Entry): Promise<void> {
    // 验收路径：直接设 100%（不走动画，保证测量即时生效）
    this.animT = null;
    this.scale = 1;
    this.offX = this.canvas.clientWidth / 2;
    this.offY = this.canvas.clientHeight / 2;
    this.draw();
    if (this.fullFrame) return;
    // 等待全尺寸就绪；15s 超时保护（加载失败时不挂死验收流程）
    await new Promise<void>((resolve) => {
      const timer = setTimeout(() => {
        this.onFullLoaded = () => {};
        resolve();
      }, 15000);
      const cb = (e: Entry) => {
        if (e.path === entry.path) {
          clearTimeout(timer);
          this.onFullLoaded = () => {};
          resolve();
        }
      };
      this.onFullLoaded = cb;
    });
  }

  benchPan(ms: number, report: (metric: string, value: number) => void): Promise<void> {
    return new Promise((resolve) => {
      const start = performance.now();
      let frames = 0;
      let winFrames = 0;
      let winStart = start;
      let minFps = Infinity;
      let dir = 1;
      const step = () => {
        const now = performance.now();
        if (now - start >= ms) {
          report("P7_pan_avg_fps", frames / ((now - start) / 1000));
          report("P7_pan_min_1s_fps", minFps === Infinity ? 0 : minFps);
          resolve();
          return;
        }
        if (now - winStart >= 500 && dir === 1) dir = -1;
        else if (now - winStart >= 1000) {
          minFps = Math.min(minFps, winFrames);
          winFrames = 0;
          winStart = now;
          dir = 1;
        }
        this.panBy(dir * 42, dir * 13);
        frames++;
        winFrames++;
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
  }
}

// ── 帧加载 ─────────────────────────────
/** 同一 URL 的并发请求共享同一 Promise（避免 show 与 loadFull 重复 fetch） */
const inflight = new Map<string, Promise<Frame>>();

export function loadFrame(entry: Entry, maxDim: number, page: number, exposure: number): Promise<Frame> {
  const url = imageUrl(entry, maxDim, page, exposure);
  const existing = inflight.get(url);
  if (existing) return existing;
  const p = doLoadFrame(entry, maxDim, url).finally(() => inflight.delete(url));
  inflight.set(url, p);
  return p;
}

async function doLoadFrame(entry: Entry, maxDim: number, url: string): Promise<Frame> {
  let resp: Response;
  try {
    resp = await fetch(url);
  } catch (err) {
    void ipc.logBench(`viewer_fetch_error_${entry.name}`, maxDim);
    console.error("image fetch 失败", url, err);
    throw err;
  }
  if (!resp.ok) throw new Error(`image 请求失败: ${resp.status}`);
  const blob = await resp.blob();
  const mime = resp.headers.get("Content-Type") ?? "";

  const naturalHeader = resp.headers.get("X-PixLens-Natural");
  let natural: { w: number; h: number } | null = null;
  if (naturalHeader) {
    const [w, h] = naturalHeader.split("x").map(Number);
    if (w > 0 && h > 0) natural = { w, h };
  }
  const pagesHeader = Number(resp.headers.get("X-PixLens-Pages") ?? "0");
  const pages = pagesHeader > 0 ? pagesHeader : null;

  const animated = entry.ext === "gif";
  const domOnly = animated || mime === "image/svg+xml";
  if (domOnly || mime === "image/avif") {
    // GIF（动画）/SVG/AVIF 用 <img>（AVIF 的 ImageBitmap 支持不稳定，统一走 img）
    const img = await loadImage(url);
    const nw = natural?.w ?? img.naturalWidth;
    const nh = natural?.h ?? img.naturalHeight;
    return {
      src: { kind: "img", img },
      naturalW: nw,
      naturalH: nh,
      nativeW: img.naturalWidth,
      isFull: maxDim === 0 || img.naturalWidth >= nw,
      pages,
    };
  }
  const bmp = await createImageBitmap(blob);
  const nw = natural?.w ?? bmp.width;
  const nh = natural?.h ?? bmp.height;
  return {
    src: { kind: "bitmap", bmp },
    naturalW: nw,
    naturalH: nh,
    nativeW: bmp.width,
    isFull: maxDim === 0 || bmp.width >= nw,
    pages,
  };
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.decoding = "async";
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("图片加载失败"));
    img.src = url;
  });
}

export { IMAGE_BASE };
