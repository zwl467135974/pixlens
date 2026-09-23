/** 双图对比视图
 *
 * 同一变换（缩放/平移/旋转/翻转）同时作用于两张图——分屏对照同一局部的差异。
 * 坐标语义与查看器一致：off 为面板中心相对偏移，缩放以光标为中心
 * （off' = q - f·(q-off) 在旋转/翻转下依然成立，两面板共用同一组值）。
 * LOD：预览先行（maxDim=2560），显示分辨率超过源分辨率后异步升级全尺寸帧
 * （与查看器共用 image:// 通道与 loadFrame，含请求合并）。
 */
import { type Entry } from "../ipc";
import { loadFrame, PREVIEW_MAX_DIM, type Frame } from "../viewer/viewer";

const MIN_SCALE = 0.02;
const MAX_SCALE = 40;

interface Panel {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  head: HTMLElement;
  entry: Entry | null;
  frame: Frame | null;
  fullToken: number;
}

export class CompareView {
  private root: HTMLElement;
  private panels: Panel[] = [];
  private zoomLabel: HTMLElement;
  /** 共享变换 */
  private scale = 1;
  private offX = 0;
  private offY = 0;
  private rot = 0;
  private flipH = false;
  private flipV = false;
  private open_ = false;
  private dragging = false;
  private drawScheduled = false;

  constructor(container: HTMLElement) {
    const root = document.createElement("div");
    root.id = "compare";
    root.className = "hidden";
    root.innerHTML = `
      <div class="cmp-body">
        <div class="cmp-panel"><canvas></canvas><div class="cmp-head"></div></div>
        <div class="cmp-divider"></div>
        <div class="cmp-panel"><canvas></canvas><div class="cmp-head"></div></div>
      </div>
      <div id="cmp-bar">
        <button data-act="fit" title="适应窗口（0）">适应</button>
        <button data-act="100" title="100%（1）">100%</button>
        <span class="vsep"></span>
        <button data-act="rotl" title="左旋 90°（L）">⟲</button>
        <button data-act="rotr" title="右旋 90°（R）">⟳</button>
        <button data-act="fliph" title="水平翻转">⇋</button>
        <button data-act="swap" title="交换左右">⇄ 交换</button>
        <span class="vsep"></span>
        <button data-act="close" title="关闭（Esc）">✕ 关闭</button>
        <span id="cmp-zoom" class="zoom"></span>
      </div>`;
    container.appendChild(root);
    this.root = root;
    this.panels = [...root.querySelectorAll<HTMLDivElement>(".cmp-panel")].map((el) => {
      const canvas = el.querySelector("canvas") as HTMLCanvasElement;
      return {
        canvas,
        ctx: canvas.getContext("2d", { alpha: false })!,
        head: el.querySelector(".cmp-head") as HTMLElement,
        entry: null,
        frame: null,
        fullToken: 0,
      };
    });
    this.zoomLabel = root.querySelector("#cmp-zoom") as HTMLElement;
    this.bindInput();
    (root.querySelector("#cmp-bar") as HTMLElement).addEventListener("click", (ev) => {
      const act = (ev.target as HTMLElement).closest("button")?.dataset.act;
      if (!act) return;
      if (act === "fit") this.fit();
      else if (act === "100") this.one();
      else if (act === "rotl") this.rotate(-90);
      else if (act === "rotr") this.rotate(90);
      else if (act === "fliph") { this.flipH = !this.flipH; this.schedule(); }
      else if (act === "swap") this.swap();
      else if (act === "close") this.close();
    });
    window.addEventListener("resize", () => {
      if (this.open_) this.schedule();
    });
  }

  get isOpen(): boolean {
    return this.open_;
  }

  open(a: Entry, b: Entry): void {
    this.open_ = true;
    this.rot = 0;
    this.flipH = false;
    this.flipV = false;
    this.root.classList.remove("hidden");
    this.loadPanel(0, a);
    this.loadPanel(1, b);
    // 两图预览到齐后按"较大者适配"定初始缩放（保证双方都完整可见）
    void Promise.all([this.panelReady(0), this.panelReady(1)]).then(() => {
      if (!this.open_) return;
      this.fit();
    });
  }

  close(): void {
    this.open_ = false;
    this.root.classList.add("hidden");
    for (const p of this.panels) {
      p.fullToken++;
      p.entry = null;
      p.frame = null;
      p.head.textContent = "";
    }
  }

  private panelReady(i: number): Promise<void> {
    const p = this.panels[i];
    return new Promise((resolve) => {
      const check = () => (p.frame || p.entry === null ? resolve() : setTimeout(check, 60));
      check();
    });
  }

  private async loadPanel(i: number, entry: Entry): Promise<void> {
    const p = this.panels[i];
    p.entry = entry;
    p.frame = null;
    p.head.textContent = `${entry.name} · 加载中…`;
    try {
      const frame = await loadFrame(entry, PREVIEW_MAX_DIM, 0, 1);
      if (p.entry !== entry || !this.open_) return;
      p.frame = frame;
      this.schedule();
      this.upgradeLod(i);
    } catch {
      // 失败也置空 entry：panelReady 轮询终止，另一张仍能正常 fit
      if (p.entry === entry) {
        p.head.textContent = `${entry.name} · 加载失败`;
        p.entry = null;
      }
    }
  }

  /** LOD 升级：显示分辨率超过源分辨率时换全尺寸帧（token 防串扰） */
  private upgradeLod(i: number): void {
    const p = this.panels[i];
    if (!p.frame || p.frame.isFull || !p.entry) return;
    const disp = Math.max(p.frame.naturalW, p.frame.naturalH) * this.scale;
    if (disp <= Math.max(p.frame.nativeW, p.frame.nativeW) * 1.05) return;
    const entry = p.entry;
    const token = ++p.fullToken;
    void loadFrame(entry, 0, 0, 1)
      .then((frame) => {
        if (token !== p.fullToken || p.entry !== entry || !this.open_) return;
        p.frame = frame;
        this.schedule();
      })
      .catch(() => {});
  }

  private swap(): void {
    const [a, b] = this.panels;
    [a.entry, b.entry] = [b.entry, a.entry];
    [a.frame, b.frame] = [b.frame, a.frame];
    a.fullToken++;
    b.fullToken++;
    this.schedule();
  }

  /** 适应窗口：取两图各自适配比例的较小值（都完整可见），旋转/偏移复位 */
  private fit(): void {
    let s = 1;
    for (const p of this.panels) {
      if (!p.frame) continue;
      const pw = p.canvas.clientWidth || 1;
      const ph = p.canvas.clientHeight || 1;
      const fs = Math.min(pw / p.frame.naturalW, ph / p.frame.naturalH);
      s = Math.min(s, fs);
    }
    this.scale = Math.max(MIN_SCALE, s);
    this.offX = 0;
    this.offY = 0;
    this.rot = 0;
    this.schedule();
  }

  /** 100% 原始像素 */
  private one(): void {
    this.scale = 1;
    this.offX = 0;
    this.offY = 0;
    this.schedule();
  }

  private rotate(deg: number): void {
    this.rot = (this.rot + deg + 360) % 360;
    this.offX = 0;
    this.offY = 0; // 与查看器一致：旋转保持缩放、居中复位
    this.schedule();
  }

  /** 面板坐标下以光标为中心缩放（两面板共用 off，天然同步） */
  private zoomAt(panelIdx: number, cx: number, cy: number, factor: number): void {
    const p = this.panels[panelIdx];
    const q = { x: cx - p.canvas.clientWidth / 2, y: cy - p.canvas.clientHeight / 2 };
    const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, this.scale * factor));
    const f = next / this.scale;
    this.offX = q.x - f * (q.x - this.offX);
    this.offY = q.y - f * (q.y - this.offY);
    this.scale = next;
    this.schedule();
  }

  private bindInput(): void {
    for (let i = 0; i < this.panels.length; i++) {
      const p = this.panels[i];
      p.canvas.addEventListener("wheel", (ev) => {
        if (!this.open_) return;
        ev.preventDefault();
        const rect = p.canvas.getBoundingClientRect();
        this.zoomAt(i, ev.clientX - rect.left, ev.clientY - rect.top, ev.deltaY < 0 ? 1.25 : 1 / 1.25);
      }, { passive: false });
      p.canvas.addEventListener("pointerdown", (ev) => {
        if (!this.open_) return;
        this.dragging = true;
        p.canvas.setPointerCapture(ev.pointerId);
      });
      p.canvas.addEventListener("pointermove", (ev) => {
        if (!this.open_ || !this.dragging) return;
        this.offX += ev.movementX;
        this.offY += ev.movementY;
        this.schedule();
      });
      p.canvas.addEventListener("pointerup", () => (this.dragging = false));
      p.canvas.addEventListener("dblclick", () => {
        if (this.open_) this.fit();
      });
    }
    window.addEventListener("keydown", (ev) => {
      if (!this.open_) return;
      const k = ev.key;
      if (k === "Escape") this.close();
      else if (k === "+" || k === "=") this.zoomAt(0, this.panels[0].canvas.clientWidth / 2, this.panels[0].canvas.clientHeight / 2, 1.25);
      else if (k === "-") this.zoomAt(0, this.panels[0].canvas.clientWidth / 2, this.panels[0].canvas.clientHeight / 2, 1 / 1.25);
      else if (k === "0") this.fit();
      else if (k === "1") this.one();
      else if (k === "r" || k === "R") this.rotate(90);
      else if (k === "l" || k === "L") this.rotate(-90);
    });
  }

  private schedule(): void {
    if (this.drawScheduled) return;
    this.drawScheduled = true;
    requestAnimationFrame(() => {
      this.drawScheduled = false;
      if (!this.open_) return;
      for (let i = 0; i < this.panels.length; i++) this.drawPanel(i);
      this.zoomLabel.textContent = `${(this.scale * 100).toFixed(0)}%`;
    });
  }

  private drawPanel(i: number): void {
    const p = this.panels[i];
    const dpr = window.devicePixelRatio || 1;
    const cw = p.canvas.clientWidth;
    const ch = p.canvas.clientHeight;
    if (p.canvas.width !== Math.round(cw * dpr) || p.canvas.height !== Math.round(ch * dpr)) {
      p.canvas.width = Math.round(cw * dpr);
      p.canvas.height = Math.round(ch * dpr);
    }
    const ctx = p.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = "#0d0d12";
    ctx.fillRect(0, 0, cw, ch);
    const f = p.frame;
    if (!f || !p.entry) return;
    ctx.save();
    ctx.translate(cw / 2 + this.offX, ch / 2 + this.offY);
    ctx.rotate((this.rot * Math.PI) / 180);
    ctx.scale(this.flipH ? -this.scale : this.scale, this.flipV ? -this.scale : this.scale);
    ctx.imageSmoothingEnabled = this.scale <= Math.max(f.nativeW / f.naturalW, 1);
    ctx.imageSmoothingQuality = "high";
    const src = f.src.kind === "bitmap" ? f.src.bmp : f.src.img;
    ctx.drawImage(src, -f.naturalW / 2, -f.naturalH / 2, f.naturalW, f.naturalH);
    ctx.restore();
    p.head.textContent = `${p.entry.name} · ${f.naturalW}×${f.naturalH}`;
    this.upgradeLod(i);
  }
}
