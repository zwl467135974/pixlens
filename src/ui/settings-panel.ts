/** 设置面板：缓存上限 / 默认排序 / 主题（doc/05 M7） */
import { ipc, type AppSettings } from "../ipc";

export class SettingsPanel {
  private root: HTMLElement;

  constructor(
    container: HTMLElement,
    private onApply: (s: AppSettings) => void,
  ) {
    const root = document.createElement("div");
    root.id = "settings-panel";
    root.className = "hidden";
    root.innerHTML = `
      <div class="bp-head"><span>设置</span><button id="sp-close" title="关闭">✕</button></div>
      <div class="ep-sec">
        <div class="ep-title">磁盘缓存上限</div>
        <label class="ep-slider"><select id="sp-cache">
          <option value="256">256 MB</option>
          <option value="512">512 MB</option>
          <option value="1024" selected>1 GB（默认）</option>
          <option value="2048">2 GB</option>
          <option value="4096">4 GB</option>
          <option value="8192">8 GB</option>
        </select></label>
        <div class="ep-dim">缩略图与查看器预览缓存（%LOCALAPPDATA%\\com.pixlens.app\\thumbs），修改即时生效</div>
      </div>
      <div class="ep-sec">
        <div class="ep-title">默认排序</div>
        <label class="ep-slider"><select id="sp-sort">
          <option value="name" selected>文件名（自然序）</option>
          <option value="mtime">修改时间</option>
          <option value="size">大小</option>
          <option value="type">类型</option>
        </select></label>
      </div>
      <div class="ep-sec">
        <div class="ep-title">主题</div>
        <label class="ep-slider"><select id="sp-theme">
          <option value="dark" selected>深色</option>
          <option value="light">浅色</option>
        </select></label>
      </div>
      <div id="sp-status" class="ep-dim"></div>`;
    container.appendChild(root);
    this.root = root;
    root.querySelector("#sp-close")!.addEventListener("click", () => this.close());
    for (const id of ["#sp-cache", "#sp-sort", "#sp-theme"]) {
      root.querySelector(id)!.addEventListener("change", () => void this.save());
    }
  }

  async open(): Promise<void> {
    try {
      const s = await ipc.getSettings();
      (this.root.querySelector("#sp-cache") as HTMLSelectElement).value = String(s.cacheLimitMb);
      (this.root.querySelector("#sp-sort") as HTMLSelectElement).value = s.defaultSort;
      (this.root.querySelector("#sp-theme") as HTMLSelectElement).value = s.theme;
    } catch {
      /* 默认值兜底 */
    }
    this.root.classList.remove("hidden");
    requestAnimationFrame(() => this.root.classList.add("in"));
  }

  close(): void {
    this.root.classList.remove("in");
    window.setTimeout(() => this.root.classList.add("hidden"), 260);
  }

  private async save(): Promise<void> {
    const s: AppSettings = {
      cacheLimitMb: Number((this.root.querySelector("#sp-cache") as HTMLSelectElement).value),
      defaultSort: (this.root.querySelector("#sp-sort") as HTMLSelectElement).value,
      theme: (this.root.querySelector("#sp-theme") as HTMLSelectElement).value,
    };
    try {
      const sizeMb = await ipc.setSettings(s);
      (this.root.querySelector("#sp-status") as HTMLElement).textContent =
        `已保存（当前缓存 ${sizeMb.toFixed(0)} MB）`;
      this.onApply(s);
    } catch (e) {
      (this.root.querySelector("#sp-status") as HTMLElement).textContent = `保存失败：${e}`;
    }
  }
}
