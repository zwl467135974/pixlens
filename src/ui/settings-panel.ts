/** 设置面板：缓存上限 / 默认排序 / 主题 / 系统关联（doc/05 M7） */
import { ipc, type AppSettings, type AssocStatus } from "../ipc";

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
      <div class="ep-sec">
        <div class="ep-title">默认看图软件</div>
        <div class="ep-dim" id="sp-assoc-state">检查中…</div>
        <div class="sp-assoc-btns">
          <button id="sp-assoc-register" class="sp-btn">注册为看图候选</button>
          <button id="sp-assoc-default" class="sp-btn">设为默认…</button>
          <button id="sp-assoc-remove" class="sp-btn">取消注册并还原</button>
        </div>
        <div class="ep-dim">注册后 PixLens 出现在所有图片格式（含 PSD / RAW，33 种）的"打开方式"与系统"默认应用"列表。Windows 10/11 规定默认应用需在系统设置中确认——点"设为默认…"跳转后选择 PixLens 即可；取消注册会还原全部改动。</div>
      </div>
      <div id="sp-status" class="ep-dim"></div>`;
    container.appendChild(root);
    this.root = root;
    root.querySelector("#sp-close")!.addEventListener("click", () => this.close());
    for (const id of ["#sp-cache", "#sp-sort", "#sp-theme"]) {
      root.querySelector(id)!.addEventListener("change", () => void this.save());
    }
    root.querySelector("#sp-assoc-register")!.addEventListener("click", () => void this.assocAction("register"));
    root.querySelector("#sp-assoc-default")!.addEventListener("click", () => void ipc.openDefaultApps());
    root.querySelector("#sp-assoc-remove")!.addEventListener("click", () => void this.assocAction("unregister"));
  }

  private async assocAction(kind: "register" | "unregister"): Promise<void> {
    const status = this.root.querySelector("#sp-assoc-state") as HTMLElement;
    const err = this.root.querySelector("#sp-status") as HTMLElement;
    status.textContent = kind === "register" ? "注册中…" : "还原中…";
    try {
      if (kind === "register") await ipc.assocRegister();
      else await ipc.assocUnregister();
      err.textContent = "";
      await this.refreshAssoc();
    } catch (e) {
      status.textContent = "操作失败";
      err.textContent = `关联操作失败：${e}`;
    }
  }

  private async refreshAssoc(): Promise<void> {
    const state = this.root.querySelector("#sp-assoc-state") as HTMLElement;
    const registerBtn = this.root.querySelector("#sp-assoc-register") as HTMLButtonElement;
    const defaultBtn = this.root.querySelector("#sp-assoc-default") as HTMLButtonElement;
    const removeBtn = this.root.querySelector("#sp-assoc-remove") as HTMLButtonElement;
    let s: AssocStatus | null = null;
    try { s = await ipc.assocStatus(); } catch { /* 保持按钮可用兜底 */ }
    if (!s) { state.textContent = "状态不可用"; return; }
    if (s.registered) {
      const n = s.defaultFor.length;
      state.textContent = n > 0
        ? `已注册候选 · ${n} 个格式已是系统默认（${s.defaultFor.slice(0, 6).join("、")}${n > 6 ? " 等" : ""}）`
        : "已注册为候选（在系统设置中可一键设为默认）";
    } else {
      state.textContent = "未注册——双击图片仍由系统原看图软件打开";
    }
    registerBtn.disabled = s.registered;
    defaultBtn.disabled = !s.registered;
    removeBtn.disabled = !s.registered;
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
    void this.refreshAssoc();
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
