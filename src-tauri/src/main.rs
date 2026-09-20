// 注意：M1~M6 保留控制台输出（[perf]/[bench] 日志用于验收测量），
// M7 发布工程时再评估是否加 windows_subsystem = "windows"。
fn main() {
    pixlens_lib::run()
}
