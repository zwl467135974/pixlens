pub mod codecs;
mod edit;
mod exif;
mod batch;
mod scan;
mod settings;
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
    /// 双击关联文件启动时待打开的文件路径
    pub launch_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    pub folder: String,
    /// 验收模式：启动时清空缩略图缓存（测冷缓存指标）
    pub clear: bool,
    /// 验收模式：跑完后自动退出进程
    pub exit: bool,
    /// 全量终验：追加 P9（5 分钟滚动内存）/ P12（缓存上限生效）等长耗时项
    pub full: bool,
}

/// 进程启动时刻（P1 冷启动埋点基准）
fn boot_instant() -> std::time::Instant {
    static BOOT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *BOOT.get_or_init(std::time::Instant::now)
}

/// 双击关联文件启动：第一个"存在的图片文件"参数
fn parse_launch_file() -> Option<String> {
    for a in std::env::args().skip(1) {
        if a.starts_with("--") || a.starts_with("-") {
            continue;
        }
        let p = std::path::Path::new(&a);
        if p.is_file() {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if scan::is_image_ext(&ext) {
                return Some(a);
            }
        }
    }
    None
}

/// bench 配置来源：CLI 参数（--bench=PATH --bench-clear --bench-exit）
/// 或环境变量（PIXLENS_BENCH / PIXLENS_BENCH_CLEAR / PIXLENS_BENCH_EXIT / PIXLENS_BENCH_FULL，便于 tauri dev 传递）
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
        full: has("--bench-full", "PIXLENS_BENCH_FULL"),
    })
}

#[tauri::command]
fn get_bench_config(state: tauri::State<AppState>) -> Option<BenchConfig> {
    // P1 冷启动：进程启动 → 前端首个 IPC（≈首屏可交互）
    println!(
        "[bench] P1_boot_to_first_ipc_ms = {:.0}",
        boot_instant().elapsed().as_secs_f64() * 1000.0
    );
    state.bench.clone()
}

#[tauri::command]
fn get_launch_file(state: tauri::State<AppState>) -> Option<String> {
    state.launch_file.clone()
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

/// 进程树内存（字节）：应用 + WebView2 子进程（两级），P9/P10 测量用；
/// 同时打印逐进程分解（定位内存构成）
#[tauri::command]
fn bench_memory() -> Result<u64, String> {
    let pid = std::process::id();
    let script = format!(
        "$p={pid}; $ws=0; $rows=@(); \
         try{{ $proc=Get-Process -Id $p -ErrorAction Stop; $ws+=$proc.WorkingSet64; \
              $rows+=\"app({pid})`t$([math]::Round($proc.WorkingSet64/1MB,0))MB\" }}catch{{}}; \
         $lvl1=Get-CimInstance Win32_Process -Filter \"ParentProcessId=$p\" | Select -ExpandProperty ProcessId; \
         foreach($k in @($lvl1)){{ try{{ $proc=Get-Process -Id $k -ErrorAction Stop; $ws+=$proc.WorkingSet64; \
              $rows+=\"lv1($k)`t$([math]::Round($proc.WorkingSet64/1MB,0))MB\" }}catch{{}}; \
         $lvl2=Get-CimInstance Win32_Process -Filter \"ParentProcessId=$k\" | Select -ExpandProperty ProcessId; \
         foreach($g in @($lvl2)){{ try{{ $proc=Get-Process -Id $g -ErrorAction Stop; $ws+=$proc.WorkingSet64; \
              $rows+=\"lv2($g)`t$([math]::Round($proc.WorkingSet64/1MB,0))MB\" }}catch{{}} }} }}; \
         Write-Output $ws; $rows | ForEach-Object {{ Write-Output $_ }}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let total_line = lines.next().unwrap_or("").trim().to_string();
    for l in lines {
        let l = l.trim();
        if !l.is_empty() {
            println!("[mem] {l}");
        }
    }
    total_line
        .parse::<u64>()
        .map_err(|e| format!("解析内存输出失败: {e} ({total_line})"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    boot_instant(); // P1 基准
    let bench = parse_bench_args();
    let launch_file = parse_launch_file();
    if bench.is_some() {
        println!("[bench] PixLens 验收模式启动");
    } else if let Some(f) = &launch_file {
        println!("[launch] 关联文件启动: {f}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // 解码并行池 = 物理核数（doc/03 §2）
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(num_cpus::get_physical())
                .build_global();
            // 设置加载：缓存上限恢复 + 主题/排序由前端读取
            let s = settings::load(app.handle());
            let state = AppState {
                thumb: {
                    let mut t = thumb::ThumbState::new(app.handle());
                    t.max_cache_bytes
                        .store(s.cache_limit_mb as u64 * 1024 * 1024, std::sync::atomic::Ordering::Relaxed);
                    t
                },
                watcher: Mutex::new(None),
                bench,
                launch_file,
            };
            // 启动时异步做一次 LRU 淘汰，防止缓存超限
            let dir = state.thumb.cache_dir.clone();
            let max = state.thumb.max_cache_bytes.load(std::sync::atomic::Ordering::Relaxed);
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
                    // 缩略图不进浏览器内存缓存（P9 内存有界）：统一走自有磁盘缓存
                    .header("Cache-Control", "no-store")
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
                    // 查看器帧由前端内存 LRU 管理，浏览器缓存一并关闭以约束内存
                    .header("Cache-Control", "no-store")
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
            batch::batch_rename_preview,
            batch::batch_rename_apply,
            batch::batch_convert,
            batch::batch_cancel,
            edit::edit_apply,
            exif::read_exif,
            settings::get_settings,
            settings::set_settings,
            get_bench_config,
            get_launch_file,
            bench_clear_cache,
            bench_done,
            bench_memory,
            log_bench
        ])
        .run(tauri::generate_context!())
        .expect("PixLens 启动失败");
}
