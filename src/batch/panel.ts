/** 批量处理面板：重命名（实时预览/冲突检测）+ 转换/压缩/缩放（进度+取消）（doc/01 F4） */
import { ipc, type BatchProgress, type RenamePair } from "../ipc";
import { listen } from "@tauri-apps/api/event";

export class BatchPanel {
  private root: HTMLElement;
  private previewBox: HTMLElement;
  private conflictBox: HTMLElement;
  private renameGo: HTMLButtonElement;
  private templateInput: HTMLInputElement;
  private startInput: HTMLInputElement;
  private progressWrap: HTMLElement;
  private barFill: HTMLElement;
  private progressText: HTMLElement;
  private cancelBtn: HTMLButtonElement;
  private convertGo: HTMLButtonElement;

  private currentPreview: RenamePair[] = [];
  private jobId = 0;
  private unlisten: (() => void) | null = null;
  private previewTimer = 0;

  constructor(
    container: HTMLElement,
    private getPaths: () => string[],
    private onClose: () => void,
  ) {
    const root = document.createElement("div");
    root.id = "batch-panel";
    root.className = "hidden";
    root.innerHTML = `
      <div class="bp-head">
        <span>批量处理</span>
        <button id="bp-close" title="关闭">✕</button>
      </div>
      <div class="bp-tabs">
        <button data-tab="rename" class="active">重命名</button>
        <button data-tab="convert">转换 / 压缩 / 缩放</button>
      </div>
      <div id="bp-rename" class="bp-tab">
        <label class="bp-field">模板
          <input id="bp-template" value="{原名}_{序号:4}" spellcheck="false" />
        </label>
        <label class="bp-field">起始序号
          <input id="bp-start" type="number" value="1" min="0" />
        </label>
        <div class="bp-tokens">可用标记：{原名} {序号} {序号:4} {日期}(文件修改日)</div>
        <div id="bp-preview" class="bp-preview"></div>
        <div id="bp-conflict" class="bp-conflict"></div>
        <button id="bp-rename-go" class="btn primary bp-go">执行重命名</button>
      </div>
      <div id="bp-convert" class="bp-tab hidden">
        <label class="bp-field">输出格式
          <select id="bp-format">
            <option value="jpg">JPG</option>
            <option value="png">PNG</option>
            <option value="webp">WebP（无损）</option>
          </select>
        </label>
        <label class="bp-field">质量 <span id="bp-quality-v">85</span>
          <input id="bp-quality" type="range" min="1" max="100" value="85" />
        </label>
        <label class="bp-field">缩放
          <select id="bp-scale-mode">
            <option value="none">不缩放</option>
            <option value="longest">最长边 (px)</option>
            <option value="percent">百分比 (%)</option>
          </select>
          <input id="bp-scale-value" type="number" value="1920" min="1" />
        </label>
        <label class="bp-field">输出位置
          <select id="bp-out">
            <option value="subdir">pixlens_output/ 子目录（保留原图）</option>
            <option value="overwrite">覆盖原图</option>
          </select>
        </label>
        <button id="bp-convert-go" class="btn primary bp-go">开始转换</button>
        <div id="bp-progress-wrap" class="hidden">
          <div class="bp-bar"><div id="bp-bar-fill"></div></div>
          <div id="bp-progress-text"></div>
          <button id="bp-cancel" class="btn">取消</button>
        </div>
      </div>`;
    container.appendChild(root);
    this.root = root;
    this.previewBox = root.querySelector("#bp-preview")!;
    this.conflictBox = root.querySelector("#bp-conflict")!;
    this.renameGo = root.querySelector("#bp-rename-go")!;
    this.templateInput = root.querySelector("#bp-template")!;
    this.startInput = root.querySelector("#bp-start")!;
    this.progressWrap = root.querySelector("#bp-progress-wrap")!;
    this.barFill = root.querySelector("#bp-bar-fill")!;
    this.progressText = root.querySelector("#bp-progress-text")!;
    this.cancelBtn = root.querySelector("#bp-cancel")!;
    this.convertGo = root.querySelector("#bp-convert-go")!;

    root.querySelector("#bp-close")!.addEventListener("click", () => this.close());
    root.querySelectorAll<HTMLButtonElement>(".bp-tabs button").forEach((b) =>
      b.addEventListener("click", () => {
        root.querySelectorAll(".bp-tabs button").forEach((x) => x.classList.remove("active"));
        b.classList.add("active");
        root.querySelector("#bp-rename")!.classList.toggle("hidden", b.dataset.tab !== "rename");
        root.querySelector("#bp-convert")!.classList.toggle("hidden", b.dataset.tab !== "convert");
      }),
    );

    const refresh = () => {
      window.clearTimeout(this.previewTimer);
      this.previewTimer = window.setTimeout(() => void this.refreshPreview(), 200);
    };
    this.templateInput.addEventListener("input", refresh);
    this.startInput.addEventListener("input", refresh);

    this.renameGo.addEventListener("click", () => void this.applyRename());
    this.convertGo.addEventListener("click", () => void this.startConvert());
    this.cancelBtn.addEventListener("click", () => void this.cancel());

    const q = root.querySelector("#bp-quality") as HTMLInputElement;
    q.addEventListener("input", () => {
      root.querySelector("#bp-quality-v")!.textContent = q.value;
    });
  }

  open(): void {
    this.root.classList.remove("hidden");
    void this.refreshPreview();
  }

  close(): void {
    this.root.classList.add("hidden");
    this.onClose();
  }

  private async refreshPreview(): Promise<void> {
    const paths = this.getPaths();
    if (!paths.length) {
      this.previewBox.innerHTML = "<div class='bp-empty'>未选择文件</div>";
      this.renameGo.disabled = true;
      return;
    }
    try {
      const pairs = await ipc.renamePreview(paths, this.templateInput.value, Number(this.startInput.value) || 1);
      this.currentPreview = pairs;
      const conflicts = pairs.filter((p) => p.conflict);
      const rows = pairs
        .slice(0, 30)
        .map(
          (p) =>
            `<div class="bp-row${p.conflict ? " bp-row-conflict" : ""}" title="${p.conflict ? p.reason : ""}">` +
            `<span class="bp-old">${p.name}</span><span class="bp-arrow">→</span><span class="bp-new">${p.newName}</span></div>`,
        )
        .join("");
      this.previewBox.innerHTML =
        rows + (pairs.length > 30 ? `<div class="bp-more">…共 ${pairs.length} 项</div>` : "");
      this.conflictBox.textContent = conflicts.length
        ? `⚠ ${conflicts.length} 项冲突将被跳过（${conflicts[0].reason}）`
        : "";
      this.renameGo.disabled = pairs.length === conflicts.length;
      this.renameGo.textContent = `执行重命名（${pairs.length - conflicts.length} 项）`;
    } catch (e) {
      this.previewBox.innerHTML = `<div class="bp-empty">${e}</div>`;
      this.renameGo.disabled = true;
    }
  }

  private async applyRename(): Promise<void> {
    const valid = this.currentPreview.filter((p) => !p.conflict);
    if (!valid.length) return;
    this.renameGo.disabled = true;
    this.renameGo.textContent = "执行中…";
    const pairs: [string, string][] = valid.map((p) => {
      const dir = p.path.replace(/[\\/][^\\/]+$/, "");
      return [p.path, `${dir}\\${p.newName}`];
    });
    try {
      const [renamed, errors] = await ipc.renameApply(pairs);
      this.conflictBox.textContent = `完成：${renamed} 改名成功${errors.length ? `，${errors.length} 失败` : ""}`;
      // fs-changed 会自动刷新网格
      this.currentPreview = [];
      this.previewBox.innerHTML = "";
    } catch (e) {
      this.conflictBox.textContent = `失败：${e}`;
    }
    this.renameGo.disabled = false;
    void this.refreshPreview();
  }

  private async startConvert(): Promise<void> {
    const paths = this.getPaths();
    if (!paths.length) return;
    const root = this.root;
    const format = (root.querySelector("#bp-format") as HTMLSelectElement).value;
    const quality = Number((root.querySelector("#bp-quality") as HTMLInputElement).value);
    const scaleMode = (root.querySelector("#bp-scale-mode") as HTMLSelectElement).value;
    const scaleValue = Number((root.querySelector("#bp-scale-value") as HTMLInputElement).value) || 0;
    const outPolicy = (root.querySelector("#bp-out") as HTMLSelectElement).value;

    this.convertGo.disabled = true;
    this.progressWrap.classList.remove("hidden");
    this.barFill.style.width = "0%";
    this.progressText.textContent = "启动中…";
    try {
      this.jobId = await ipc.batchConvert(paths, { format, quality, scaleMode, scaleValue, outPolicy });
      if (!this.unlisten) {
        this.unlisten = await listen<BatchProgress>("batch-progress", (ev) => this.onProgress(ev.payload));
      }
    } catch (e) {
      this.progressText.textContent = `启动失败：${e}`;
      this.convertGo.disabled = false;
    }
  }

  private onProgress(p: BatchProgress): void {
    if (p.jobId !== this.jobId) return;
    const pct = p.total ? Math.round(((p.done + p.failed) / p.total) * 100) : 0;
    this.barFill.style.width = `${pct}%`;
    this.progressText.textContent = p.finished
      ? p.canceled
        ? `已取消：完成 ${p.done} / ${p.total}，失败 ${p.failed}`
        : `完成：成功 ${p.done} / ${p.total}，失败 ${p.failed}`
      : `${p.done + p.failed} / ${p.total}（失败 ${p.failed}）`;
    if (p.finished) {
      this.convertGo.disabled = false;
      this.convertGo.textContent = "再次转换";
    }
  }

  private async cancel(): Promise<void> {
    if (this.jobId) await ipc.batchCancel(this.jobId);
    this.progressText.textContent = "正在取消…";
  }
}
