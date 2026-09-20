/** 虚拟滚动缩略图墙（doc/01 F1、doc/04 P4：只渲染可见行 ±2 行缓冲） */
import { thumbUrl, type Entry } from "../ipc";

const BUFFER_ROWS = 2;

export class Grid {
  private el: HTMLElement;
  private spacer: HTMLElement;
  private tiles = new Map<number, HTMLElement>();
  private entries: Entry[] = [];
  private gap = 12;
  private cols = 1;
  private rowH = 172;
  private raf = 0;

  tileSize = 160;
  /** 缩略图加载完成回调（bench 埋点用） */
  onThumbLoaded: (e: Entry) => void = () => {};
  /** tile 点击回调（进入查看器） */
  onTileClick: (e: Entry) => void = () => {};

  constructor(container: HTMLElement) {
    this.el = container;
    this.spacer = document.createElement("div");
    this.spacer.className = "grid-spacer";
    this.el.appendChild(this.spacer);
    this.el.addEventListener("scroll", () => this.schedule(), { passive: true });
    new ResizeObserver(() => this.relayout()).observe(this.el);
  }

  setEntries(list: Entry[]): void {
    this.entries = list;
    this.relayout();
  }

  setTileSize(size: number): void {
    this.tileSize = size;
    this.relayout();
  }

  scrollBy(px: number): void {
    this.el.scrollTop += px;
  }

  get entryCount(): number {
    return this.entries.length;
  }

  private relayout(): void {
    const w = this.el.clientWidth;
    this.cols = Math.max(1, Math.floor((w - this.gap) / (this.tileSize + this.gap)));
    this.rowH = this.tileSize + this.gap;
    const rows = Math.ceil(this.entries.length / this.cols);
    this.spacer.style.height = `${rows * this.rowH}px`;
    this.clearTiles();
    this.render();
  }

  private schedule(): void {
    if (!this.raf) {
      this.raf = requestAnimationFrame(() => {
        this.raf = 0;
        this.render();
      });
    }
  }

  private clearTiles(): void {
    for (const t of this.tiles.values()) t.remove();
    this.tiles.clear();
  }

  private render(): void {
    if (!this.entries.length) return;
    const scrollTop = this.el.scrollTop;
    const firstRow = Math.max(0, Math.floor(scrollTop / this.rowH) - BUFFER_ROWS);
    const visibleRows = Math.ceil(this.el.clientHeight / this.rowH) + 1;
    const lastRow = Math.min(
      Math.ceil(this.entries.length / this.cols) - 1,
      firstRow + visibleRows + BUFFER_ROWS,
    );
    const first = firstRow * this.cols;
    const last = Math.min(this.entries.length - 1, (lastRow + 1) * this.cols - 1);

    for (const [idx, tile] of this.tiles) {
      if (idx < first || idx > last) {
        tile.remove();
        this.tiles.delete(idx);
      }
    }
    for (let i = first; i <= last; i++) {
      if (this.tiles.has(i)) continue;
      const e = this.entries[i];
      if (!e) continue;
      this.tiles.set(i, this.makeTile(i, e));
    }
  }

  private makeTile(idx: number, e: Entry): HTMLElement {
    const tile = document.createElement("div");
    tile.className = "tile";
    tile.style.width = `${this.tileSize}px`;
    tile.style.height = `${this.tileSize}px`;
    tile.style.transform = `translate(${(idx % this.cols) * (this.tileSize + this.gap)}px, ${
      Math.floor(idx / this.cols) * this.rowH
    }px)`;
    tile.title = e.name;
    tile.addEventListener("click", () => this.onTileClick(e));
    const img = document.createElement("img");
    img.decoding = "async";
    img.alt = e.name;
    img.addEventListener("load", () => this.onThumbLoaded(e));
    img.src = thumbUrl(e, this.tileSize);
    tile.appendChild(img);
    this.spacer.appendChild(tile);
    return tile;
  }
}
