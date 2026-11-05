/** 排序 / 过滤 / 增量补丁（doc/01 F1） */
import type { Entry, FsChanged } from "./ipc";

export type SortKey = "name" | "mtime" | "size" | "type";
export type SortDir = 1 | -1;

/** 自然排序：数字段按数值比较（img_2 < img_10）；
 * 文字分层对齐资源管理器——非 CJK 在前，汉字按拼音（极乐楼 < 蚂蚁消除 < 我超勇往 < 消除星星）。
 * 注意 ICU 的 zh-CN collator 会把汉字排在拉丁之前（与 Explorer 相反），不能整串直用；
 * 只在 CJK 对 CJK 的字符比较时用它的拼音序。 */
const CJK_RE = /[\u2E80-\u9FFF\uF900-\uFAFF\uFE30-\uFE4F\u{20000}-\u{3FFFF}]/u;
const collCJK = new Intl.Collator("zh-Hans-CN", { numeric: true, sensitivity: "base" });

function isCJK(ch: string): boolean {
  return CJK_RE.test(ch);
}

/** 文本块比较：逐字符——CJK 恒大于非 CJK；同为 CJK 用拼音 collator；否则码点（已 lowercase） */
function cmpText(x: string, y: string): number {
  const xa = Array.from(x);
  const ya = Array.from(y);
  const n = Math.min(xa.length, ya.length);
  for (let i = 0; i < n; i++) {
    const cx = xa[i];
    const cy = ya[i];
    if (cx === cy) continue;
    const kx = isCJK(cx) ? 1 : 0;
    const ky = isCJK(cy) ? 1 : 0;
    if (kx !== ky) return kx - ky;
    if (kx === 1) return collCJK.compare(cx, cy) || xa.length - ya.length;
    return cx < cy ? -1 : 1;
  }
  return xa.length - ya.length;
}

export function naturalCompare(a: string, b: string): number {
  const chunk = (s: string): (number | string)[] => {
    const out: (number | string)[] = [];
    const re = /(\d+)|(\D+)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(s))) {
      out.push(m[1] ? parseInt(m[1], 10) : m[2]);
    }
    return out;
  };
  const ax = chunk(a.toLowerCase());
  const bx = chunk(b.toLowerCase());
  const n = Math.min(ax.length, bx.length);
  for (let i = 0; i < n; i++) {
    const x = ax[i];
    const y = bx[i];
    if (x === y) continue;
    if (typeof x === "number" && typeof y === "number") return x < y ? -1 : 1;
    if (typeof x === typeof y) return cmpText(String(x), String(y));
    return typeof x === "number" ? -1 : 1;
  }
  return ax.length - bx.length;
}

export class AppState {
  all: Entry[] = [];
  view: Entry[] = [];
  sortKey: SortKey = "name";
  sortDir: SortDir = 1;
  filter = "";

  setAll(entries: Entry[]): void {
    this.all = entries;
    this.recompute();
  }

  setSort(key: SortKey, dir: SortDir): void {
    this.sortKey = key;
    this.sortDir = dir;
    this.recompute();
  }

  setFilter(text: string): void {
    this.filter = text.trim().toLowerCase();
    this.recompute();
  }

  /** 应用 fs-changed 增量补丁 */
  patch(p: FsChanged): void {
    if (p.removed.length) {
      const gone = new Set(p.removed);
      this.all = this.all.filter((e) => !gone.has(e.path));
    }
    if (p.created.length) {
      const known = new Set(this.all.map((e) => e.path));
      for (const e of p.created) {
        if (!known.has(e.path)) this.all.push(e);
      }
    }
    if (p.updated.length) {
      const upd = new Map(p.updated.map((e) => [e.path, e]));
      this.all = this.all.map((e) => upd.get(e.path) ?? e);
    }
    this.recompute();
  }

  recompute(): void {
    const f = this.filter;
    let list = f ? this.all.filter((e) => e.name.toLowerCase().includes(f)) : this.all.slice();
    const dir = this.sortDir;
    const key = this.sortKey;
    list.sort((a, b) => dir * compareByKey(key, a, b) || naturalCompare(a.name, b.name) * dir);
    this.view = list;
  }
}

function compareByKey(key: SortKey, a: Entry, b: Entry): number {
  switch (key) {
    case "mtime":
      return a.mtime - b.mtime;
    case "size":
      return a.size - b.size;
    case "type":
      // Explorer"类型"列是描述串（"JPEG 文件"等）而非扩展名——用同款字符串排序对齐
      return cmpText(a.typeName || a.ext, b.typeName || b.ext);
    case "name":
    default:
      return naturalCompare(a.name, b.name);
  }
}
