// 生产构建隐藏控制台窗口；stdout 在被管道/重定向时依然可用（验收埋点不受影响）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pixlens_lib::run()
}
