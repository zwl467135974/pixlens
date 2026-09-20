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

interface Frame {
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
        <button data-act="close" title="关闭（Esc）">✕</button>
        <span id="viewer-zoom" class="zoom"></span>
      </div>`;
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

  open(path: string): void {
    const entries = this.getEntries();
    const i = entries.findIndex((e) => e.path === path);
    if (i < 0) return;
    this.open_ = true;
    this.root.classList.remove("hidden");
    this.resize();
    void this.show(i);
  }

  close(): void {
    this.open_ = false;
    this.stopSlideshow();
    if (document.fullscreenElement) void document.exitFullscreen();
    this.root.classList.add("hidden");
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
    this.fit();
    this.draw();
    this.updateVariantUi();
    this.prefetch(i);
    void this.loadFull(entry, token);
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

  // ── 变换 ─────────────────────────────
  private fit(): void {
    const f = this.effective();
    if (!f) return;
    const rotated = this.rot % 180 !== 0;
    const w = rotated ? f.naturalH : f.naturalW;
    const h = rotated ? f.naturalW : f.naturalH;
    const cw = this.canvas.clientWidth;
    const ch = this.canvas.clientHeight;
    this.scale = Math.min(cw / w, ch / h);
    this.offX = cw / 2;
    this.offY = ch / 2;
  }

  private one(): void {
    this.scale = 1;
    this.offX = this.canvas.clientWidth / 2;
    this.offY = this.canvas.clientHeight / 2;
  }

  zoomAt(cx: number, cy: number, factor: number): void {
    const ns = Math.min(MAX_SCALE, Math.max(MIN_SCALE, this.scale * factor));
    const f = ns / this.scale;
    this.scale = ns;
    this.offX = cx - f * (cx - this.offX);
    this.offY = cy - f * (cy - this.offY);
    this.draw();
  }

  panBy(dx: number, dy: number): void {
    this.offX += dx;
    this.offY += dy;
    this.draw();
  }

  rotate(deg: number): void {
    this.rot = (this.rot + deg + 360) % 360;
    this.fit();
    this.draw();
  }

  flip(axis: "h" | "v"): void {
    if (axis === "h") this.flipH = !this.flipH;
    else this.flipV = !this.flipV;
    this.draw();
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

  private draw(): void {
    const entry = this.cur;
    const f = this.effective();
    const dpr = window.devicePixelRatio || 1;
    const ctx = this.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
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
      this.zoomLabel.textContent = `${Math.round(this.scale * 100)}%`;
    } else {
      this.gifLayer.classList.add("hidden");
      ctx.translate(this.offX, this.offY);
      ctx.rotate((this.rot * Math.PI) / 180);
      const sx = this.flipH ? -this.scale : this.scale;
      const sy = this.flipV ? -this.scale : this.scale;
      ctx.scale(sx, sy);
      // >100% 时关闭平滑（像素视图），<100% 时开启（缩小时抗锯齿）
      ctx.imageSmoothingEnabled = this.scale < 1;
      ctx.drawImage(f.src.bmp, -f.naturalW / 2, -f.naturalH / 2, f.naturalW, f.naturalH);
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
    this.infoPanel.innerHTML = `
      <div class="row name">${e.name}</div>
      <div class="row">${f.naturalW} × ${f.naturalH} px · ${e.ext.toUpperCase()}${pageInfo}</div>
      <div class="row">${fmtSize(e.size)} · 缩放 ${Math.round(this.scale * 100)}%</div>
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
      dragging = true;
      lastX = ev.clientX;
      lastY = ev.clientY;
    });
    window.addEventListener("mousemove", (ev) => {
      if (!dragging || !this.open_) return;
      this.panBy(ev.clientX - lastX, ev.clientY - lastY);
      lastX = ev.clientX;
      lastY = ev.clientY;
    });
    window.addEventListener("mouseup", () => {
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
      } else if (k === "Escape") {
        this.close();
      }
    });
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
      case "slideshow": this.toggleSlideshow(); break;
      case "info":
        this.infoVisible = !this.infoVisible;
        if (!this.infoVisible) this.infoPanel.classList.add("hidden");
        this.draw();
        break;
      case "close": this.close(); break;
    }
  }

  private async toggleFullscreen(): Promise<void> {
    if (document.fullscreenElement) {
      await document.exitFullscreen();
    } else {
      await this.root.requestFullscreen();
    }
    this.resize();
    this.fit();
    this.draw();
  }

  toggleSlideshow(): void {
    if (this.slideshowTimer) this.stopSlideshow();
    else this.startSlideshow();
  }

  private startSlideshow(): void {
    if (!document.fullscreenElement) void this.root.requestFullscreen();
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
    this.one();
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

function loadFrame(entry: Entry, maxDim: number, page: number, exposure: number): Promise<Frame> {
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
