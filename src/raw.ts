/** 相机 RAW 解码（libraw-wasm：LibRaw 的 WebAssembly 构建）
 *
 * - wasm ~1.4MB 独立 chunk，首次遇到 RAW 才加载（启动零成本）
 * - 库内置 Web Worker，解码不阻塞 UI
 * - 策略：相机白平衡 + 相机色彩矩阵 + sRGB 8bit——还原拍摄观感，不做显影调节
 * - 缩略图/预览走 halfSize（1/2 尺寸，快约 4 倍），全尺寸仅 100% 查看时按需
 */
export const RAW_EXTS = new Set([
  "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "pef", "srw",
  "x3f", "erf", "mrw", "kdc", "dcr", "iiq", "rwl", "gpr", "3fr",
]);

export function isRawExt(ext: string): boolean {
  return RAW_EXTS.has(ext.toLowerCase());
}

interface LibRawImage {
  width: number;
  height: number;
  data: Uint8Array | Uint16Array;
}
interface LibRawInstance {
  open(bytes: BufferSource, settings?: Record<string, unknown>): Promise<void>;
  imageData(): Promise<LibRawImage | undefined>;
}
type LibRawFactory = (opts: { locateFile?: (p: string, d: string) => string }) => Promise<{
  LibRaw: new () => LibRawInstance;
}>;

let modPromise: Promise<{ LibRaw: new () => LibRawInstance }> | null = null;
function libraw(): Promise<{ LibRaw: new () => LibRawInstance }> {
  modPromise ??= (async () => {
    // ?url 让 vite 把 wasm 作为资源产出；Emscripten 胶水的默认相对解析在打包后无效
    const [{ default: factory }, { default: wasmUrl }] = await Promise.all([
      import("libraw-wasm/dist/libraw.js"),
      import("libraw-wasm/dist/libraw.wasm?url"),
    ]);
    return (factory as unknown as LibRawFactory)({ locateFile: () => wasmUrl });
  })();
  return modPromise;
}

export interface RawFrame {
  bmp: ImageBitmap;
  width: number;
  height: number;
}

/** 解码一帧 RAW；half=半尺寸（缩略图/预览），返回 ImageBitmap 与实际输出尺寸 */
export async function decodeRaw(bytes: ArrayBuffer, half: boolean): Promise<RawFrame> {
  const mod = await libraw();
  const lr = new mod.LibRaw();
  await lr.open(new Uint8Array(bytes), {
    halfSize: half,
    useCameraWb: true,
    useCameraMatrix: 3,
    outputColor: 1, // sRGB
    outputBps: 8,
  });
  const img = await lr.imageData();
  if (!img || !img.width || !img.height) throw new Error("RAW 解码失败");
  // LibRaw 输出 RGB(3ch) 或 RGBA(4ch) → 统一 RGBA
  const px = img.width * img.height;
  const rgba = new Uint8ClampedArray(px * 4);
  const data = img.data;
  if (data.length === px * 4) {
    for (let i = 0; i < px * 4; i += 4) {
      rgba[i] = data[i] as number;
      rgba[i + 1] = data[i + 1] as number;
      rgba[i + 2] = data[i + 2] as number;
      rgba[i + 3] = 255;
    }
  } else {
    for (let s = 0, d = 0; s < Math.min(data.length, px * 3); s += 3, d += 4) {
      rgba[d] = data[s] as number;
      rgba[d + 1] = data[s + 1] as number;
      rgba[d + 2] = data[s + 2] as number;
      rgba[d + 3] = 255;
    }
  }
  const bmp = await createImageBitmap(new ImageData(rgba, img.width, img.height));
  return { bmp, width: img.width, height: img.height };
}

/** 缩略图场景：解码半尺寸并缩到目标边长内（控制 LRU 中位图体积） */
export async function decodeRawThumb(bytes: ArrayBuffer, maxDim: number): Promise<ImageBitmap> {
  const { bmp } = await decodeRaw(bytes, true);
  if (bmp.width <= maxDim && bmp.height <= maxDim) return bmp;
  const scale = Math.min(maxDim / bmp.width, maxDim / bmp.height);
  const w = Math.max(1, Math.round(bmp.width * scale));
  const h = Math.max(1, Math.round(bmp.height * scale));
  const small = await createImageBitmap(bmp, { resizeWidth: w, resizeHeight: h });
  bmp.close();
  return small;
}
