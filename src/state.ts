/** 排序 / 过滤 / 增量补丁（doc/01 F1） */
import type { Entry, FsChanged } from "./ipc";

export type SortKey = "name" | "mtime" | "size" | "type";
export type SortDir = 1 | -1;

/** 自然排序：数字段按数值比较（img_2 < img_10） */
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
    if (typeof x === typeof y) return String(x) < String(y) ? -1 : 1;
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
      return a.ext < b.ext ? -1 : a.ext > b.ext ? 1 : naturalCompare(a.name, b.name);
    case "name":
    default:
      return naturalCompare(a.name, b.name);
  }
}
