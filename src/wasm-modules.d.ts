/** libraw-wasm 的 Emscripten 产物没有类型声明；wasm 的 ?url 导入是 Vite 特性 */
declare module "libraw-wasm/dist/libraw.js" {
  const factory: (opts: { locateFile?: (path: string, dir: string) => string }) => Promise<unknown>;
  export default factory;
}
declare module "libraw-wasm/dist/libraw.wasm?url" {
  const url: string;
  export default url;
}
