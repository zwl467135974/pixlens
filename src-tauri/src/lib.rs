pub mod codecs;
mod scan;
mod thumb;
mod viewer;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::Manager;

/// 全局共享状态
pub struct AppState {
    pub thumb: thumb::ThumbState,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub bench: Option<BenchConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    pub folder: String,
    /// 验收模式：启动时清空缩略图缓存（测冷缓存指标）
    pub clear: bool,
    /// 验收模式：跑完后自动退出进程
    pub exit: bool,
}

/// bench 配置来源：CLI 参数（--bench=PATH --bench-clear --bench-exit）
/// 或环境变量（PIXLENS_BENCH / PIXLENS_BENCH_CLEAR / PIXLENS_BENCH_EXIT，便于 tauri dev 传递）
fn parse_bench_args() -> Option<BenchConfig> {
    let args: Vec<String> = std::env::args().collect();
    let folder = args
        .iter()
        .find_map(|a| a.strip_prefix("--bench=").map(String::from))
        .or_else(|| std::env::var("PIXLENS_BENCH").ok())?;
    let has = |flag: &str, env: &str| {
        args.iter().any(|a| a == flag) || std::env::var(env).map(|v| v == "1").unwrap_or(false)
    };
    Some(BenchConfig {
        folder,
        clear: has("--bench-clear", "PIXLENS_BENCH_CLEAR"),
        exit: has("--bench-exit", "PIXLENS_BENCH_EXIT"),
    })
}

#[tauri::command]
fn get_bench_config(state: tauri::State<AppState>) -> Option<BenchConfig> {
    state.bench.clone()
}

#[tauri::command]
fn bench_clear_cache(state: tauri::State<AppState>) -> Result<(), String> {
    thumb::wipe_cache(&state.thumb.cache_dir)
}

#[tauri::command]
fn bench_done(app: tauri::AppHandle) {
    println!("[bench] done");
    app.exit(0);
}

/// 前端埋点统一转发到 stdout，便于外部脚本采集
#[tauri::command]
fn log_bench(metric: String, value: f64) {
    println!("[bench] {} = {:.1}", metric, value);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bench = parse_bench_args();
    if bench.is_some() {
        println!("[bench] PixLens 验收模式启动");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // 解码并行池 = 物理核数（doc/03 §2）
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(num_cpus::get_physical())
                .build_global();
            let state = AppState {
                thumb: thumb::ThumbState::new(app.handle()),
                watcher: Mutex::new(None),
                bench,
            };
            // 启动时异步做一次 LRU 淘汰，防止缓存超限
            let dir = state.thumb.cache_dir.clone();
            let max = state.thumb.max_cache_bytes;
            std::thread::spawn(move || thumb::evict_if_needed(&dir, max));
            app.manage(state);
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, request, responder| {
            // 每请求一线程 + 信号量限流：协议回调不做重活，解码在线程中执行
            let app = ctx.app_handle().clone();
            let uri = request.uri().clone();
            std::thread::spawn(move || {
                let out = {
                    let state = app.state::<AppState>();
                    thumb::handle(&state.thumb, &uri)
                };
                let len = out.bytes.len();
                let resp = tauri::http::Response::builder()
                    .header("Content-Type", out.mime)
                    .header("Content-Length", len)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(out.bytes)
                    .unwrap_or_else(|e| {
                        tauri::http::Response::builder()
                            .status(500)
                            .body(e.to_string().into_bytes())
                            .unwrap()
                    });
                responder.respond(resp);
            });
        })
        .register_asynchronous_uri_scheme_protocol("image", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let uri = request.uri().clone();
            if cfg!(debug_assertions) {
                println!("[viewer] 收到请求 {uri}");
            }
            std::thread::spawn(move || {
                let out = {
                    let state = app.state::<AppState>();
                    viewer::handle(&state.thumb, &uri)
                };
                let mut builder = tauri::http::Response::builder()
                    .header("Content-Type", out.mime.clone())
                    .header("Content-Length", out.bytes.len())
                    .header("Access-Control-Allow-Origin", "*")
                    // 自定义头必须显式暴露，跨域 fetch 才能读到
                    .header("Access-Control-Expose-Headers", "X-PixLens-Natural, X-PixLens-Pages");
                if let Some((w, h)) = out.natural {
                    builder = builder.header("X-PixLens-Natural", format!("{w}x{h}"));
                }
                if let Some(pages) = out.pages {
                    builder = builder.header("X-PixLens-Pages", pages.to_string());
                }
                let resp = builder
                    .body(out.bytes)
                    .unwrap_or_else(|e| {
                        tauri::http::Response::builder()
                            .status(500)
                            .body(e.to_string().into_bytes())
                            .unwrap()
                    });
                responder.respond(resp);
            });
        })
        .invoke_handler(tauri::generate_handler![
            scan::scan_folder,
            get_bench_config,
            bench_clear_cache,
            bench_done,
            log_bench
        ])
        .run(tauri::generate_context!())
        .expect("PixLens 启动失败");
}
