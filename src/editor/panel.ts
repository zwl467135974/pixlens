/** 基础编辑面板：裁剪/旋转翻转/亮度对比度饱和度/滤镜（doc/01 F5）
 *
 * 预览 = 成品：色彩/滤镜走 CSS filter（公式与 edit.rs 完全一致），
 * 旋转/翻转直接驱动查看器视图变换，裁剪用画布框选（逆变换回原图 ROI）。
 * 保存时把全部操作一次性交给 Rust 应用（原分辨率管线）。
 */
import { invoke } from "@tauri-apps/api/core";
import type { Entry } from "../ipc";
import type { Viewer } from "../viewer/viewer";

export interface EditOps {
  crop: { x: number; y: number; w: number; h: number } | null;
  rotate: number;
  flipH: boolean;
  flipV: boolean;
  brightness: number;
  contrast: number;
  saturation: number;
  filter: string;
}

const defaultOps = (): EditOps => ({
  crop: null,
  rotate: 0,
  flipH: false,
  flipV: false,
  brightness: 0,
  contrast: 0,
  saturation: 0,
  filter: "none",
});

/** 与 edit.rs apply_color 逐公式一致（CSS filter 链序：brightness→contrast→saturate→滤镜） */
function cssFilterOf(ops: EditOps): string {
  const parts: string[] = [];
  if (ops.brightness) parts.push(`brightness(${1 + ops.brightness / 100})`);
  if (ops.contrast) parts.push(`contrast(${1 + ops.contrast / 100})`);
  if (ops.saturation) parts.push(`saturate(${1 + ops.saturation / 100})`);
  if (ops.filter === "gray") parts.push("grayscale(1)");
  if (ops.filter === "sepia") parts.push("sepia(1)");
  if (ops.filter === "invert") parts.push("invert(1)");
  return parts.length ? parts.join(" ") : "none";
}

export class EditorPanel {
  private root: HTMLElement;
  private ops = defaultOps();
  private cropBtn: HTMLButtonElement;
  private cropInfo: HTMLElement;
  private resultBox: HTMLElement;
  private saveBtn: HTMLButtonElement;

  constructor(
    container: HTMLElement,
    private viewer: Viewer,
  ) {
    const root = document.createElement("div");
    root.id = "editor-panel";
    root.className = "hidden";
    root.innerHTML = `
      <div class="bp-head"><span>编辑</span><button id="ep-close" title="关闭">✕</button></div>
      <div class="ep-note">所见即所得：当前视图的旋转/翻转会一并保存</div>
      <div class="ep-sec">
        <div class="ep-title">裁剪 / 几何</div>
        <div class="ep-row">
          <button id="ep-crop" class="btn">▭ 框选裁剪</button>
          <button id="ep-crop-clear" class="btn" disabled>清除</button>
        </div>
        <div id="ep-crop-info" class="ep-dim"></div>
        <div class="ep-row">
          <button id="ep-rotl" class="btn" title="左旋 90°">⟲</button>
          <button id="ep-rotr" class="btn" title="右旋 90°">⟳</button>
          <button id="ep-fliph" class="btn" title="水平翻转">⇋</button>
          <button id="ep-flipv" class="btn" title="垂直翻转">⇵</button>
          <button id="ep-reset" class="btn" title="重置几何">重置</button>
        </div>
      </div>
      <div class="ep-sec">
        <div class="ep-title">调节（实时预览）</div>
        <label class="ep-slider">亮度 <input id="ep-bri" type="range" min="-100" max="100" value="0" /><span>0</span></label>
        <label class="ep-slider">对比度 <input id="ep-con" type="range" min="-100" max="100" value="0" /><span>0</span></label>
        <label class="ep-slider">饱和度 <input id="ep-sat" type="range" min="-100" max="100" value="0" /><span>0</span></label>
      </div>
      <div class="ep-sec">
        <div class="ep-title">滤镜</div>
        <div class="ep-row" id="ep-filters">
          <button data-f="none" class="btn active">无</button>
          <button data-f="gray" class="btn">灰度</button>
          <button data-f="sepia" class="btn">复古</button>
          <button data-f="invert" class="btn">反色</button>
        </div>
      </div>
      <div class="ep-sec">
        <div class="ep-title">保存</div>
        <label class="ep-slider">质量 <input id="ep-q" type="range" min="1" max="100" value="92" /><span>92</span></label>
        <div class="ep-row">
          <label class="ep-radio"><input type="radio" name="ep-out" value="copy" checked /> 另存副本（保留原图）</label>
          <label class="ep-radio"><input type="radio" name="ep-out" value="overwrite" /> 覆盖原图</label>
        </div>
        <button id="ep-save" class="btn primary">保存</button>
        <div id="ep-result" class="ep-dim"></div>
      </div>`;
    container.appendChild(root);
    this.root = root;
    this.cropBtn = root.querySelector("#ep-crop")!;
    this.cropInfo = root.querySelector("#ep-crop-info")!;
    this.resultBox = root.querySelector("#ep-result")!;
    this.saveBtn = root.querySelector("#ep-save")!;

    root.querySelector("#ep-close")!.addEventListener("click", () => this.close());
    this.cropBtn.addEventListener("click", () => {
      this.resultBox.textContent = "在图片上拖拽框选裁剪区域（Esc 取消）";
      this.viewer.enterCropMode((rect) => this.onCropSelected(rect));
    });
    root.querySelector("#ep-crop-clear")!.addEventListener("click", () => {
      this.ops.crop = null;
      this.cropInfo.textContent = "";
      (root.querySelector("#ep-crop-clear") as HTMLButtonElement).disabled = true;
    });
    root.querySelector("#ep-rotl")!.addEventListener("click", () => this.setRotate(-90));
    root.querySelector("#ep-rotr")!.addEventListener("click", () => this.setRotate(90));
    root.querySelector("#ep-fliph")!.addEventListener("click", () => {
      this.ops.flipH = !this.ops.flipH;
      this.viewer.flip("h");
    });
    root.querySelector("#ep-flipv")!.addEventListener("click", () => {
      this.ops.flipV = !this.ops.flipV;
      this.viewer.flip("v");
    });
    root.querySelector("#ep-reset")!.addEventListener("click", () => {
      this.ops.rotate = 0;
      this.ops.flipH = false;
      this.ops.flipV = false;
      this.viewer.rot = 0;
      this.viewer.draw();
    });
    for (const [id, key] of [
      ["#ep-bri", "brightness"],
      ["#ep-con", "contrast"],
      ["#ep-sat", "saturation"],
    ] as const) {
      const input = root.querySelector(id) as HTMLInputElement;
      const label = input.nextElementSibling!;
      input.addEventListener("input", () => {
        label.textContent = input.value;
        this.ops[key] = Number(input.value);
        this.applyPreview();
      });
    }
    root.querySelectorAll<HTMLButtonElement>("#ep-filters button").forEach((b) =>
      b.addEventListener("click", () => {
        root.querySelectorAll("#ep-filters button").forEach((x) => x.classList.remove("active"));
        b.classList.add("active");
        this.ops.filter = b.dataset.f!;
        this.applyPreview();
      }),
    );
    const q = root.querySelector("#ep-q") as HTMLInputElement;
    q.addEventListener("input", () => {
      q.nextElementSibling!.textContent = q.value;
    });
    this.saveBtn.addEventListener("click", () => void this.save());
  }

  open(): void {
    if (!this.viewer.isOpen) return;
    this.ops = defaultOps();
    this.resultBox.textContent = "";
    this.cropInfo.textContent = "";
    (this.root.querySelector("#ep-crop-clear") as HTMLButtonElement).disabled = true;
    // 视图状态作为几何操作基线（WYSIWYG）
    this.ops.rotate = this.viewer.rot;
    this.ops.flipH = this.viewer.flipH;
    this.ops.flipV = this.viewer.flipV;
    this.root.classList.remove("hidden");
    this.applyPreview();
  }

  close(): void {
    this.root.classList.add("hidden");
    this.viewer.exitCropMode();
    this.viewer.setPreviewFilter("none");
  }

  private applyPreview(): void {
    this.viewer.setPreviewFilter(cssFilterOf(this.ops));
  }

  private setRotate(deg: number): void {
    this.ops.rotate = (this.ops.rotate + deg + 360) % 360;
    this.viewer.rotate(deg);
  }

  private onCropSelected(rect: { x1: number; y1: number; x2: number; y2: number }): void {
    const p1 = this.viewer.screenToImage(rect.x1, rect.y1);
    const p2 = this.viewer.screenToImage(rect.x2, rect.y2);
    if (!p1 || !p2) return;
    const entry = this.viewer.currentEntry();
    this.viewer.exitCropMode();
    if (!entry) return;
    const x = Math.max(0, Math.floor(Math.min(p1.x, p2.x)));
    const y = Math.max(0, Math.floor(Math.min(p1.y, p2.y)));
    const w = Math.ceil(Math.abs(p2.x - p1.x));
    const h = Math.ceil(Math.abs(p2.y - p1.y));
    if (w < 4 || h < 4) {
      this.resultBox.textContent = "裁剪区太小，已忽略";
      return;
    }
    this.ops.crop = { x, y, w, h };
    this.cropInfo.textContent = `裁剪区 ${w} × ${h} px（原图坐标 ${x},${y}）`;
    (this.root.querySelector("#ep-crop-clear") as HTMLButtonElement).disabled = false;
    this.resultBox.textContent = "";
  }

  private async save(): Promise<void> {
    const entry: Entry | null = this.viewer.currentEntry();
    if (!entry) return;
    const overwrite =
      (this.root.querySelector("input[name=ep-out]:checked") as HTMLInputElement)?.value === "overwrite";
    const quality = Number((this.root.querySelector("#ep-q") as HTMLInputElement).value);
    this.saveBtn.disabled = true;
    this.saveBtn.textContent = "保存中…";
    try {
      const out = await invoke<string>("edit_apply", {
        src: entry.path,
        ops: this.ops,
        overwrite,
        quality,
      });
      this.resultBox.textContent = overwrite ? `已覆盖：${out}` : `已另存：${out}`;
      if (overwrite) {
        // 原图已变：清缓存重载当前图
        this.viewer.reloadCurrent();
      }
    } catch (e) {
      this.resultBox.textContent = `保存失败：${e}`;
    }
    this.saveBtn.disabled = false;
    this.saveBtn.textContent = "保存";
  }
}
