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
  /** 解码位图 LRU（键 = thumb URL） */
  private bitmaps = new Map<string, ImageBitmap>();
  private inflight = new Set<string>();
  private selected = new Set<string>();

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
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top + this.el.scrollTop;
    const pitch = this.tileSize + this.gap;
    const col = Math.floor(x / pitch);
    const row = Math.floor((y - this.gap) / this.rowH);
    if (col < 0 || col >= this.cols || row < 0) return null;
    const idx = row * this.cols + col;
    const e = this.entries[idx];
    if (!e) return null;
    // 命中 tile 实际矩形（含间隙宽容 2px）
    const tx = this.gap + col * pitch;
    const ty = this.gap + row * this.rowH;
    if (x < tx - 2 || x > tx + this.tileSize + 2 || y < ty - 2 || y > ty + this.tileSize + 2) {
      return null;
    }
    return e;
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
        // 位图
        const key = thumbUrl(e, this.tileSize);
        const bmp = this.bitmaps.get(key);
        if (bmp) {
          const scale = Math.min(this.tileSize / bmp.width, this.tileSize / bmp.height);
          const dw = bmp.width * scale;
          const dh = bmp.height * scale;
          ctx.drawImage(bmp, x + (this.tileSize - dw) / 2, y + (this.tileSize - dh) / 2, dw, dh);
        } else {
          this.loadThumb(e, key);
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
        const bmp = await createImageBitmap(blob);
        // LRU 淘汰（淘汰即释放）
        this.bitmaps.delete(key);
        this.bitmaps.set(key, bmp);
        while (this.bitmaps.size > BMP_CAPACITY) {
          const oldest = this.bitmaps.keys().next().value as string;
          const old = this.bitmaps.get(oldest);
          this.bitmaps.delete(oldest);
          old?.close();
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
