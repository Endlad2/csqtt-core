//! iOS статическая библиотека для csqtt-core.
//! Собирается как статическая библиотека (.a) для iOS.
//! Без uniffi — только C FFI.

#![cfg(target_os = "ios")]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

// Импорт основного клиента
use csqtt_client::{
auth::VkAuth,
captcha::CaptchaSolver,
dispatcher::Dispatcher,
obfs::ObfsMode,
packet::PacketPool,
stats::Stats,
worker::{RuntimeParams, run_groups, parse_hashes, GroupContext, WorkerStartPacer},
events::Events,
wrap::derive_wrap_key,
dns::resolve_socket,
};

// Глобальный экземпляр токио рантайма
static TOKIO_RUNTIME: OnceLock<Runtime> = OnceLock::new();

// Глобальный CancellationToken
static CANCEL_TOKEN: OnceLock<CancellationToken> = OnceLock::new();

// Глобальный Dispatcher
static DISPATCHER: OnceLock<std::sync::Arc<Dispatcher>> = OnceLock::new();

/// Инициализация рантайма. Должна быть вызвана один раз при старте приложения.
#[no_mangle]
pub extern "C" fn csqtt_init_runtime() -> bool {
TOKIO_RUNTIME.get_or_init(|| {
tokio::runtime::Builder::new_multi_thread()
.worker_threads(4)
.enable_time()
.enable_io()
.build()
.expect("Failed to create Tokio runtime")
});

CANCEL_TOKEN.get_or_init(CancellationToken::new);
true
}

/// Запускает клиент с параметрами.
/// Принимает C-строки, возвращает 0 при успехе, -1 при ошибке.
#[no_mangle]
pub extern "C" fn csqtt_start_client(
vk_hashes: *const c_char,
peer: *const c_char,
password: *const c_char,
device_id: *const c_char,
vk_auth_mode: *const c_char,
fingerprint: *const c_char,
obfs: *const c_char,
workers: u8,
) -> i32 {
// Безопасное преобразование C-строк в Rust-строки
let vk_hashes = unsafe { CStr::from_ptr(vk_hashes).to_str().unwrap_or("") };
let peer = unsafe { CStr::from_ptr(peer).to_str().unwrap_or("") };
let password = unsafe { CStr::from_ptr(password).to_str().unwrap_or("") };
let device_id = unsafe { CStr::from_ptr(device_id).to_str().unwrap_or("") };
let vk_auth_mode = unsafe { CStr::from_ptr(vk_auth_mode).to_str().unwrap_or("vkcalls") };
let fingerprint = unsafe { CStr::from_ptr(fingerprint).to_str().unwrap_or("chrome") };
let obfs = unsafe { CStr::from_ptr(obfs).to_str().unwrap_or("audio") };

let rt = match TOKIO_RUNTIME.get() {
Some(rt) => rt,
None => return -1,
};

let cancel = match CANCEL_TOKEN.get() {
Some(cancel) => cancel,
None => return -1,
};

let hashes: Vec<String> = parse_hashes(vk_hashes)
.into_iter()
.take(6)
.collect();

if hashes.is_empty() {
println!("[iOS] No valid VK hashes provided");
return -1;
}

let workers = (workers as usize).clamp(9, 162) / 9 * 9;
let groups = workers / 9;

println!("[iOS] Starting with {} workers, {} groups", workers, groups);

rt.block_on(async {
let captcha = CaptchaSolver::new("auto", cancel.clone());
let auth = std::sync::Arc::new(VkAuth::new(
vk_auth_mode,
fingerprint,
&[],
captcha,
None,
));

let stats = std::sync::Arc::new(Stats::default());
let pool = PacketPool::new(csqtt_client::packet::packet_pool_size(workers));

let (dispatcher, _port) = match Dispatcher::start(
"127.0.0.1:0",
None,
pool.clone(),
stats.clone(),
cancel.clone(),
).await {
Ok(d) => d,
Err(e) => {
println!("[iOS] Failed to start dispatcher: {}", e);
return -1;
}
};

let _ = DISPATCHER.set(dispatcher.clone());

let params = std::sync::Arc::new(RuntimeParams {
peer: match resolve_socket(peer).await {
Ok(addr) => addr,
Err(e) => {
println!("[iOS] Failed to resolve peer: {}", e);
return -1;
}
},
turn_host: None,
turn_port: None,
hashes: hashes.into(),
wrap_key: match derive_wrap_key(password) {
Ok(key) => key,
Err(e) => {
println!("[iOS] Failed to derive wrap key: {}", e);
return -1;
}
},
mode: match ObfsMode::parse(obfs) {
Ok(m) => m,
Err(e) => {
println!("[iOS] Invalid obfs mode: {}", e);
return -1;
}
},
generation: 0,
salt: "".into(),
local_port: "0".into(),
device_id: device_id.into(),
password: password.into(),
});

let (config_tx, _config_rx) = tokio::sync::mpsc::channel::<String>(32);

let context = std::sync::Arc::new(GroupContext {
params,
auth,
dispatcher: dispatcher.clone(),
pool,
stats,
events: Events::new(false),
paused: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
config_tx,
start_pacer: std::sync::Arc::new(WorkerStartPacer::new(
Duration::from_millis(100)
)),
credential_pacer: std::sync::Arc::new(tokio::sync::Mutex::new(())),
ready_credential_tx: None,
config_sent: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
config_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
cancel: cancel.clone(),
});

let groups_handle = tokio::spawn(async move {
run_groups(groups, context).await;
});

cancel.cancelled().await;
dispatcher.shutdown().await;
let _ = groups_handle.await;

0
})
}

/// Остановка всех воркеров.
#[no_mangle]
pub extern "C" fn csqtt_stop_client() -> bool {
if let Some(cancel) = CANCEL_TOKEN.get() {
cancel.cancel();
true
} else {
false
}
}

/// Проверка статуса клиента.
#[no_mangle]
pub extern "C" fn csqtt_is_running() -> bool {
if let Some(cancel) = CANCEL_TOKEN.get() {
!cancel.is_cancelled()
} else {
false
}
}