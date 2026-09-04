// ios-core/lib.rs
#![allow(dead_code, unused_imports)]

// Импортируем модули из папки rust-client (../)
#[path = "../auth.rs"]
mod auth;
#[path = "../captcha.rs"]
mod captcha;
#[path = "../captcha_slider.rs"]
mod captcha_slider;
#[path = "../client_perf.rs"]
mod client_perf;
#[path = "../cpu_task.rs"]
mod cpu_task;
#[path = "../dispatcher.rs"]
mod dispatcher;
#[path = "../dns.rs"]
mod dns;
#[path = "../events.rs"]
mod events;
#[path = "../../shared/selective_fec.rs"]
mod selective_fec;
#[path = "../logging.rs"]
mod logging;
#[path = "../namegen.rs"]
mod namegen;
#[path = "../obfs.rs"]
mod obfs;
#[path = "../packet.rs"]
mod packet;
#[path = "../profiles.rs"]
mod profiles;
#[path = "../protocol.rs"]
mod protocol;
#[path = "../repair.rs"]
mod repair;
#[path = "../session.rs"]
mod session;
#[path = "../stats.rs"]
mod stats;
#[path = "../../shared/striped_scheduler.rs"]
mod striped_scheduler;
#[path = "../stun_codec.rs"]
mod stun_codec;
#[path = "../tun.rs"]
mod tun;
#[path = "../turn.rs"]
mod turn;
#[path = "../turn_core.rs"]
mod turn_core;
#[path = "../turn_endpoint.rs"]
mod turn_endpoint;
#[path = "../turn_stream.rs"]
mod turn_stream;
#[path = "../udp_batch.rs"]
mod udp_batch;
#[path = "../vk_js_calls.rs"]
mod vk_js_calls;
#[path = "../worker.rs"]
mod worker;
#[path = "../wrap.rs"]
mod wrap;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::Mutex;
use std::collections::VecDeque;
use std::sync::LazyLock;

// ===== Логирование через C-ABI =====

static LOG_BUFFER: LazyLock<Mutex<VecDeque<String>>> = LazyLock::new(|| {
    Mutex::new(VecDeque::with_capacity(1024))
});

static LOG_CALLBACK: Mutex<Option<unsafe extern "C" fn(*const c_char)>> = Mutex::new(None);

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_set_log_callback(callback: unsafe extern "C" fn(*const c_char)) {
    let mut guard = LOG_CALLBACK.lock().unwrap();
    *guard = Some(callback);
}

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_get_logs() -> *const c_char {
    let guard = LOG_BUFFER.lock().unwrap();
    let logs: String = guard.iter().map(|s| format!("{}\n", s)).collect();
    CString::new(logs).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_free_logs(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_clear_logs() {
    let mut guard = LOG_BUFFER.lock().unwrap();
    guard.clear();
}

pub fn push_log(line: String) {
    if let Ok(callback_guard) = LOG_CALLBACK.lock() {
        if let Some(cb) = *callback_guard {
            if let Ok(cstr) = CString::new(line.clone()) {
                unsafe { cb(cstr.as_ptr()); }
            }
        }
    }
    let mut guard = LOG_BUFFER.lock().unwrap();
    if guard.len() >= 1024 {
        guard.pop_front();
    }
    guard.push_back(line);
}

// ===== Аргументы =====

#[derive(Debug, Clone)]
pub struct CsqttConfig {
    pub peer: String,
    pub vk: String,
    pub password: String,
    pub listen: String,
    pub workers: i32,
    pub device_id: String,
    pub vk_hash_mode: String,
    pub vk_auth_mode: String,
    pub captcha_mode: String,
    pub fingerprint: String,
    pub client_ids: String,
    pub obfs: String,
    pub turn_transport: String,
    pub generation: u64,
    pub salt: String,
    pub turn: String,
    pub port: String,
    pub allow_hash_redistribution: bool,
    pub validate_vk_hashes: bool,
    pub tun_uds: String,
}

impl CsqttConfig {
    fn to_args_vec(&self) -> Vec<String> {
        let mut args = Vec::new();
        args.push(format!("--peer={}", self.peer));
        args.push(format!("--vk={}", self.vk));
        args.push(format!("--password={}", self.password));
        args.push(format!("--listen={}", self.listen));
        args.push(format!("--workers={}", self.workers));
        if !self.device_id.is_empty() && self.device_id != "unknown" {
            args.push(format!("--device-id={}", self.device_id));
        }
        if !self.vk_hash_mode.is_empty() && self.vk_hash_mode != "manual" {
            args.push(format!("--vk-hash-mode={}", self.vk_hash_mode));
        }
        if !self.vk_auth_mode.is_empty() && self.vk_auth_mode != "vkcalls" {
            args.push(format!("--vk-auth-mode={}", self.vk_auth_mode));
        }
        if !self.captcha_mode.is_empty() && self.captcha_mode != "auto" {
            args.push(format!("--captcha-mode={}", self.captcha_mode));
        }
        if !self.fingerprint.is_empty() && self.fingerprint != "chrome" {
            args.push(format!("--fingerprint={}", self.fingerprint));
        }
        if !self.client_ids.is_empty() {
            args.push(format!("--client-ids={}", self.client_ids));
        }
        if !self.obfs.is_empty() && self.obfs != "audio" {
            args.push(format!("--obfs={}", self.obfs));
        }
        if !self.turn_transport.is_empty() && self.turn_transport != "udp" {
            args.push(format!("--turn-transport={}", self.turn_transport));
        }
        if self.generation != 0 {
            args.push(format!("--gen={}", self.generation));
        }
        if !self.salt.is_empty() {
            args.push(format!("--salt={}", self.salt));
        }
        if !self.turn.is_empty() {
            args.push(format!("--turn={}", self.turn));
        }
        if !self.port.is_empty() {
            args.push(format!("--port={}", self.port));
        }
        if self.allow_hash_redistribution {
            args.push("--allow-hash-redistribution".to_string());
        }
        if self.validate_vk_hashes {
            args.push("--validate-vk-hashes".to_string());
        }
        if !self.tun_uds.is_empty() {
            args.push(format!("--tun-uds={}", self.tun_uds));
        }
        args
    }
}

// ===== Основная логика (скопирована из main.rs) =====

async fn run_client(config: CsqttConfig) -> Result<(), anyhow::Error> {
    use auth::VkAuth;
    use captcha::CaptchaSolver;
    use dispatcher::Dispatcher;
    use events::Events;
    use obfs::ObfsMode;
    use packet::{PacketPool, packet_pool_size};
    use repair::RepairState;
    use session::ShutdownCoordinator;
    use stats::Stats;
    use tokio_util::sync::CancellationToken;
    use turn_endpoint::TurnTransportMode;
    use worker::{
        GroupContext, PauseGate, RuntimeParams, WORKER_START_INTERVAL,
        WORKERS_PER_GROUP, WorkerStartPacer, parse_hashes, run_groups,
    };
    use wrap::derive_wrap_key;

    let peer = crate::dns::resolve_socket(&config.peer).await?;
    let mode = ObfsMode::parse(&config.obfs)?;
    let turn_transport = TurnTransportMode::parse(&config.turn_transport)?;
    let wrap_key = derive_wrap_key(&config.password)?;
    let session_profile = profiles::random_profile(&config.fingerprint);

    let hashes: Vec<_> = parse_hashes(&config.vk)
        .into_iter()
        .take(6)
        .collect();
    
    if hashes.is_empty() {
        anyhow::bail!("[КЛИЕНТ] Нет хешей VK");
    }

    let workers = config.workers as usize;
    let groups = workers / WORKERS_PER_GROUP;
    let cancel = CancellationToken::new();
    let captcha = CaptchaSolver::new(&config.captcha_mode, cancel.clone());
    let events = Events::from_env();
    
    let client_ids: Vec<_> = config.client_ids
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect();

    let auth = std::sync::Arc::new(VkAuth::new(
        &config.vk_auth_mode,
        session_profile,
        &client_ids,
        captcha.clone(),
        None,
    ));

    let stats = std::sync::Arc::new(Stats::default());
    let paused = std::sync::Arc::new(PauseGate::new());
    let pool = PacketPool::new(packet_pool_size(workers));
    
    let (dispatcher, local_port) = Dispatcher::start(
        &config.listen,
        None,
        pool.clone(),
        stats.clone(),
        cancel.clone(),
    ).await?;

    let local_port: std::sync::Arc<str> = std::sync::Arc::from(local_port);
    
    let params = std::sync::Arc::new(RuntimeParams {
        peer,
        turn_host: (!config.turn.is_empty()).then(|| std::sync::Arc::from(config.turn.as_str())),
        turn_port: (!config.port.is_empty()).then(|| std::sync::Arc::from(config.port.as_str())),
        turn_transport,
        hashes: hashes.into(),
        wrap_key,
        mode,
        generation: config.generation,
        salt: std::sync::Arc::from(config.salt.as_str()),
        local_port: local_port.clone(),
        device_id: std::sync::Arc::from(config.device_id.as_str()),
        password: std::sync::Arc::from(config.password.as_str()),
        workers,
    });

    let repair = RepairState::new(workers);
    let (config_tx, _config_rx) = tokio::sync::mpsc::channel::<String>(32);
    
    let context = std::sync::Arc::new(GroupContext {
        params,
        auth,
        dispatcher: dispatcher.clone(),
        pool,
        stats,
        events: events.clone(),
        paused,
        config_tx,
        start_pacer: std::sync::Arc::new(WorkerStartPacer::new(WORKER_START_INTERVAL)),
        credential_pacer: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        ready_credential_tx: None,
        config_sent: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        config_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        repair,
        shutdown: std::sync::Arc::new(ShutdownCoordinator::new()),
        cancel: cancel.clone(),
    });

    run_groups(groups, context).await;
    dispatcher.shutdown().await;
    
    Ok(())
}

// ===== C-ABI экспорт =====

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_run(
    peer: *const c_char,
    vk: *const c_char,
    password: *const c_char,
    listen: *const c_char,
    workers: c_int,
    device_id: *const c_char,
    vk_hash_mode: *const c_char,
    vk_auth_mode: *const c_char,
    captcha_mode: *const c_char,
    fingerprint: *const c_char,
    client_ids: *const c_char,
    obfs: *const c_char,
    turn_transport: *const c_char,
    generation: u64,
    salt: *const c_char,
    turn: *const c_char,
    port: *const c_char,
    allow_hash_redistribution: bool,
    validate_vk_hashes: bool,
    tun_uds: *const c_char,
) -> c_int {
    let config = CsqttConfig {
        peer: unsafe { cstr_to_string(peer) },
        vk: unsafe { cstr_to_string(vk) },
        password: unsafe { cstr_to_string(password) },
        listen: unsafe { cstr_to_string(listen) },
        workers,
        device_id: unsafe { cstr_to_string(device_id) },
        vk_hash_mode: unsafe { cstr_to_string(vk_hash_mode) },
        vk_auth_mode: unsafe { cstr_to_string(vk_auth_mode) },
        captcha_mode: unsafe { cstr_to_string(captcha_mode) },
        fingerprint: unsafe { cstr_to_string(fingerprint) },
        client_ids: unsafe { cstr_to_string(client_ids) },
        obfs: unsafe { cstr_to_string(obfs) },
        turn_transport: unsafe { cstr_to_string(turn_transport) },
        generation,
        salt: unsafe { cstr_to_string(salt) },
        turn: unsafe { cstr_to_string(turn) },
        port: unsafe { cstr_to_string(port) },
        allow_hash_redistribution,
        validate_vk_hashes,
        tun_uds: unsafe { cstr_to_string(tun_uds) },
    };

    push_log("[iOS] Starting client".to_string());

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            push_log(format!("[iOS] Runtime error: {}", e));
            return -1;
        }
    };

    rt.block_on(async {
        match run_client(config).await {
            Ok(()) => {
                push_log("[iOS] Client finished successfully".to_string());
                0
            }
            Err(e) => {
                push_log(format!("[iOS] Error: {}", e));
                -3
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_stop() -> c_int {
    push_log("[iOS] Stop called".to_string());
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn csqtt_status() -> c_int {
    1
}

// ===== Хелперы =====

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    match CStr::from_ptr(ptr).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => String::new(),
    }
}