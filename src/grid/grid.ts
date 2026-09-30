/** 虚拟滚动缩略图墙（doc/01 F1、doc/04 P4/P9）
 *
 * Canvas 位图渲染：fetch → createImageBitmap → 定容 LRU → drawImage。
 * 解码位图由我们持有并在淘汰时 close()，内存严格有界（P9 达标的关键）——
 * 不用 <img>（Chromium 对滚动过的解码位图无上限保留）。
 */
import { thumbUrl, type Entry } from "../ipc";

const BUFFER_ROWS = 2;
/** 解码位图 LRU 容量（128px 位图 ≈ 64KB，400 张 ≈ 26MB） */
const BMP_CAPACITY = 400;

export class Grid {
  private el: HTMLElement;
  private spacer: HTMLElement;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private entries: Entry[] = [];
  private gap = 12;
  private cols = 1;
  private rowH = 172;
  private raf = 0;
  private dpr = 1;
  /** 解码位图 LRU（键 = thumb URL；at = 加载完成时刻，用于淡入动效） */
  private bitmaps = new Map<string, { bmp: ImageBitmap; at: number }>();
  private inflight = new Set<string>();
  private selected = new Set<string>();
  /** hover 命中的条目索引（-1 无） */
  private hoverIdx = -1;

  tileSize = 160;
  /** tile 点击回调（选择，含修饰键信息） */
  onTileClick: (e: Entry, ev: MouseEvent) => void = () => {};
  /** tile 双击回调（进入查看器） */
  onTileOpen: (e: Entry) => void = () => {};
  /** 缩略图加载完成回调（bench 埋点用） */
  onThumbLoaded: (e: Entry) => void = () => {};

  constructor(container: HTMLElement) {
    this.el = container;
    this.spacer = document.createElement("div");
    this.spacer.className = "grid-spacer";
    this.canvas = document.createElement("canvas");
    this.canvas.id = "grid-canvas";
    this.el.appendChild(this.spacer);
    this.el.appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d")!;

    this.el.addEventListener("scroll", () => {
      // 画布贴住可视区（与 rAF 重绘同帧，无错位）
      this.canvas.style.transform = `translateY(${this.el.scrollTop}px)`;
      this.schedule();
    }, { passive: true });
    new ResizeObserver(() => this.relayout()).observe(this.el);

    this.canvas.addEventListener("click", (ev) => {
      const e = this.hitTest(ev);
      if (e) this.onTileClick(e, ev);
    });
    this.canvas.addEventListener("dblclick", (ev) => {
      const e = this.hitTest(ev);
      if (e) this.onTileOpen(e);
    });
    this.canvas.addEventListener("mousemove", (ev) => {
      const rect = this.canvas.getBoundingClientRect();
      const idx = this.hitIndexAt(ev.clientX - rect.left, ev.clientY - rect.top);
      if (idx !== this.hoverIdx) {
        this.hoverIdx = idx;
        this.canvas.style.cursor = idx >= 0 ? "pointer" : "default";
        this.schedule();
      }
    });
    this.canvas.addEventListener("mouseleave", () => {
      if (this.hoverIdx !== -1) {
        this.hoverIdx = -1;
        this.schedule();
      }
    });
  }

  setEntries(list: Entry[]): void {
    this.entries = list;
    this.relayout();
  }

  setTileSize(size: number): void {
    this.tileSize = size;
    this.relayout();
  }

  /** 更新选中态并重绘 */
  setSelected(paths: Set<string>): void {
    this.selected = paths;
    this.schedule();
  }

  /** 当前悬停项（空格快速预览目标） */
  hoverEntry(): Entry | null {
    if (this.hoverIdx >= 0 && this.hoverIdx < this.entries.length) return this.entries[this.hoverIdx];
    return null;
  }

  scrollBy(px: number): void {
    this.el.scrollTop += px;
  }

  scrollToTop(): void {
    this.el.scrollTop = 0;
  }

  isNearBottom(thresholdPx = 4000): boolean {
    return this.el.scrollTop + this.el.clientHeight >= this.el.scrollHeight - thresholdPx;
  }

  private hitTest(ev: MouseEvent): Entry | null {
    const rect = this.canvas.getBoundingClientRect();
    const idx = this.hitIndexAt(ev.clientX - rect.left, ev.clientY - rect.top);
    return idx >= 0 ? (this.entries[idx] ?? null) : null;
  }

  /** 视口坐标 → 条目索引（-1 未命中；含 2px 间隙宽容） */
  private hitIndexAt(x: number, yView: number): number {
    const y = yView + this.el.scrollTop;
    const pitch = this.tileSize + this.gap;
    const col = Math.floor(x / pitch);
    const row = Math.floor((y - this.gap) / this.rowH);
    if (col < 0 || col >= this.cols || row < 0) return -1;
    const idx = row * this.cols + col;
    if (!this.entries[idx]) return -1;
    const tx = this.gap + col * pitch;
    const ty = this.gap + row * this.rowH;
    if (x < tx - 2 || x > tx + this.tileSize + 2 || y < ty - 2 || y > ty + this.tileSize + 2) {
      return -1;
    }
    return idx;
  }

  private relayout(): void {
    const w = this.el.clientWidth;
    const h = this.el.clientHeight;
    this.dpr = window.devicePixelRatio || 1;
    this.cols = Math.max(1, Math.floor((w - this.gap) / (this.tileSize + this.gap)));
    this.rowH = this.tileSize + this.gap;
    const rows = Math.ceil(this.entries.length / this.cols);
    this.spacer.style.height = `${Math.max(rows * this.rowH + this.gap, this.el.clientHeight)}px`;
    this.canvas.width = Math.max(1, Math.round(w * this.dpr));
    this.canvas.height = Math.max(1, Math.round(h * this.dpr));
    this.canvas.style.width = `${w}px`;
    this.canvas.style.height = `${h}px`;
    this.schedule();
  }

  private schedule(): void {
    if (!this.raf) {
      this.raf = requestAnimationFrame(() => {
        this.raf = 0;
        this.render();
      });
    }
  }

  private render(): void {
    const ctx = this.ctx;
    const w = this.canvas.width;
    const h = this.canvas.height;
    const now = performance.now();
    let animating = false;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = "#1a1a1f";
    if (document.documentElement.dataset.theme === "light") {
      ctx.fillStyle = "#f2f2f6";
    }
    ctx.fillRect(0, 0, w, h);
    if (!this.entries.length) return;

    const scrollTop = this.el.scrollTop;
    const firstRow = Math.max(0, Math.floor(scrollTop / this.rowH) - BUFFER_ROWS);
    const visibleRows = Math.ceil(this.el.clientHeight / this.rowH) + 1;
    const lastRow = Math.min(
      Math.ceil(this.entries.length / this.cols) - 1,
      firstRow + visibleRows + BUFFER_ROWS,
    );

    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    const pitch = this.tileSize + this.gap;
    const tileBg = document.documentElement.dataset.theme === "light" ? "#e6e6ee" : "#2a2a30";
    for (let row = firstRow; row <= lastRow; row++) {
      for (let col = 0; col < this.cols; col++) {
        const idx = row * this.cols + col;
        const e = this.entries[idx];
        if (!e) break;
        const x = this.gap + col * pitch;
        const y = this.gap + row * this.rowH - scrollTop;
        if (y > this.el.clientHeight + this.tileSize || y < -this.tileSize - this.gap) continue;
        // 底色
        ctx.fillStyle = tileBg;
        this.roundRect(ctx, x, y, this.tileSize, this.tileSize, 6);
        ctx.fill();
        // 位图（加载完成后 160ms 淡入）
        const key = thumbUrl(e, this.tileSize);
        const rec = this.bitmaps.get(key);
        if (rec) {
          const alpha = Math.min(1, (now - rec.at) / 160);
          if (alpha < 1) animating = true; // 未完成淡入 → 下一帧继续
          const bmp = rec.bmp;
          const scale = Math.min(this.tileSize / bmp.width, this.tileSize / bmp.height);
          const dw = bmp.width * scale;
          const dh = bmp.height * scale;
          ctx.globalAlpha = alpha;
          ctx.drawImage(bmp, x + (this.tileSize - dw) / 2, y + (this.tileSize - dh) / 2, dw, dh);
          ctx.globalAlpha = 1;
        } else {
          this.loadThumb(e, key);
        }
        // 视频角标：半透明圆底 + 播放三角（无位图时也保留，便于识别）
        if (e.kind === "video") {
          const r = Math.max(10, Math.round(this.tileSize * 0.1));
          const cx = x + this.tileSize - r - 9;
          const cy = y + this.tileSize - r - 9;
          ctx.fillStyle = "rgba(12,12,16,0.62)";
          ctx.beginPath();
          ctx.arc(cx, cy, r, 0, Math.PI * 2);
          ctx.fill();
          ctx.fillStyle = "rgba(255,255,255,0.94)";
          ctx.beginPath();
          ctx.moveTo(cx - r * 0.32, cy - r * 0.48);
          ctx.lineTo(cx - r * 0.32, cy + r * 0.48);
          ctx.lineTo(cx + r * 0.56, cy);
          ctx.closePath();
          ctx.fill();
        }
        // hover 高亮（未选中时）
        if (idx === this.hoverIdx && !this.selected.has(e.path)) {
          ctx.strokeStyle = "rgba(160,155,240,0.9)";
          ctx.lineWidth = 2;
          this.roundRect(ctx, x + 1, y + 1, this.tileSize - 2, this.tileSize - 2, 6);
          ctx.stroke();
          ctx.fillStyle = "rgba(255,255,255,0.05)";
          this.roundRect(ctx, x, y, this.tileSize, this.tileSize, 6);
          ctx.fill();
        }
        // 选中态
        if (this.selected.has(e.path)) {
          ctx.strokeStyle = "#6c66e8";
          ctx.lineWidth = 3;
          this.roundRect(ctx, x + 1.5, y + 1.5, this.tileSize - 3, this.tileSize - 3, 5);
          ctx.stroke();
          ctx.fillStyle = "rgba(108,102,232,0.18)";
          this.roundRect(ctx, x, y, this.tileSize, this.tileSize, 6);
          ctx.fill();
        }
      }
    }
    // 有未完成的淡入 → 下一帧继续（动画结束自动停，不空转）
    if (animating) this.schedule();
  }

  private roundRect(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    w: number,
    h: number,
    r: number,
  ): void {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + h, r);
    ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
  }

  private loadThumb(e: Entry, key: string): void {
    if (this.inflight.has(key)) return;
    this.inflight.add(key);
    void (async () => {
      try {
        const resp = await fetch(key);
        if (!resp.ok) return;
        const blob = await resp.blob();
        // SVG：createImageBitmap 不支持矢量 blob，兜底经 <img> 栅格化（与查看器同路）
        const bmp = await createImageBitmap(blob).catch(() => decodeViaImg(blob));
        // LRU 淘汰（淘汰即释放）
        this.bitmaps.delete(key);
        this.bitmaps.set(key, { bmp, at: performance.now() });
        while (this.bitmaps.size > BMP_CAPACITY) {
          const oldest = this.bitmaps.keys().next().value as string;
          const old = this.bitmaps.get(oldest);
          this.bitmaps.delete(oldest);
          old?.bmp.close();
        }
        this.onThumbLoaded(e);
        this.schedule();
      } catch {
        /* 解码失败：保留底色占位 */
      } finally {
        this.inflight.delete(key);
      }
    })();
  }
}

/** createImageBitmap 不支持的格式（SVG 矢量）：经 <img> 解码后转位图，统一进 LRU 管线 */
async function decodeViaImg(blob: Blob): Promise<ImageBitmap> {
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    img.decoding = "async";
    img.src = url;
    await img.decode();
    if (img.naturalWidth > 0 && img.naturalHeight > 0) {
      return await createImageBitmap(img);
    }
    // 无内在尺寸的 SVG（仅 viewBox）：解析 viewBox 定尺寸，canvas 显式栅格化
    const text = await blob.text();
    const m = /viewBox\s*=\s*["']\s*[\d.eE+-]+\s+[\d.eE+-]+\s+([\d.eE+-]+)\s+([\d.eE+-]+)/.exec(text);
    const w = m ? Math.max(1, Math.round(Number(m[1]))) : 512;
    const h = m ? Math.max(1, Math.round(Number(m[2]))) : 512;
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    c.getContext("2d")!.drawImage(img, 0, 0, w, h);
    return await createImageBitmap(c);
  } finally {
    URL.revokeObjectURL(url);
  }
}
