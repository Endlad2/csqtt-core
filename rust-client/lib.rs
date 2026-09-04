// src/lib.rs
#![allow(dead_code, unused_imports)]

mod auth;
mod captcha;
mod captcha_slider;
mod client_perf;
mod cpu_task;
mod dispatcher;
mod dns;
mod events;
mod flow_frame;
mod logging;
mod namegen;
mod obfs;
mod packet;
mod profiles;
mod protocol;
mod repair;
mod selective_fec;
mod session;
mod stats;
mod striped_scheduler;
mod stun_codec;
mod tun;
mod turn;
mod turn_core;
mod turn_endpoint;
mod turn_stream;
mod udp_batch;
mod vk_js_calls;
mod wire_protocol;
mod worker;
mod wrap;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::Mutex;
use std::collections::VecDeque;

use anyhow::Result;
use clap::Parser;
use tokio_util::sync::CancellationToken;

// ===== Логирование через C-ABI =====

static LOG_BUFFER: Mutex<VecDeque<String>> = Mutex::new(VecDeque::with_capacity(1024));
static LOG_CALLBACK: Mutex<Option<extern "C" fn(*const c_char)>> = Mutex::new(None);

/// Установить callback для логов (вызывается из Swift)
#[no_mangle]
pub extern "C" fn set_log_callback(callback: extern "C" fn(*const c_char)) {
    let mut guard = LOG_CALLBACK.lock().unwrap();
    *guard = Some(callback);
}

/// Получить логи в виде строки (если callback не используется)
#[no_mangle]
pub extern "C" fn get_logs() -> *const c_char {
    let guard = LOG_BUFFER.lock().unwrap();
    let logs: String = guard.iter().map(|s| format!("{}\n", s)).collect();
    CString::new(logs).unwrap().into_raw()
}

/// Освободить память после get_logs()
#[no_mangle]
pub extern "C" fn free_logs(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)); }
    }
}

/// Очистить буфер логов
#[no_mangle]
pub extern "C" fn clear_logs() {
    let mut guard = LOG_BUFFER.lock().unwrap();
    guard.clear();
}

/// Перехват логов из logging.rs
#[macro_export]
macro_rules! log_output {
    ($($arg:tt)*) => {{
        let line = format_args!($($arg)*).to_string();
        $crate::push_log(line);
        $crate::logging::stdout(format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        let line = format_args!($($arg)*).to_string();
        $crate::push_log(format!("[ERROR] {}", line));
        $crate::logging::stderr(format_args!($($arg)*));
    }};
}

pub fn push_log(line: String) {
    // В callback
    if let Some(cb) = *LOG_CALLBACK.lock().unwrap() {
        if let Ok(cstr) = CString::new(line) {
            cb(cstr.as_ptr());
        }
    }
    // В буфер
    let mut guard = LOG_BUFFER.lock().unwrap();
    if guard.len() >= 1024 {
        guard.pop_front();
    }
    guard.push_back(line);
}

// ===== Аргументы как в бинарнике =====

#[derive(Parser, Debug, Clone)]
#[command(disable_help_flag = true)]
pub struct CArguments {
    #[arg(long, default_value = "")]
    pub turn: String,
    #[arg(long, default_value = "")]
    pub port: String,
    #[arg(long, default_value = "127.0.0.1:9000")]
    pub listen: String,
    #[arg(long, default_value = "", allow_hyphen_values = true)]
    pub vk: String,
    #[arg(long, default_value = "manual")]
    pub vk_hash_mode: String,
    #[arg(long, default_value = "")]
    pub peer: String,
    #[arg(short = 'n', long, default_value_t = 18)]
    pub workers: usize,
    #[arg(long, default_value_t = false)]
    pub allow_hash_redistribution: bool,
    #[arg(long, default_value = "unknown")]
    pub device_id: String,
    #[arg(long, default_value = "")]
    pub password: String,
    #[arg(long, default_value = "vkcalls")]
    pub vk_auth_mode: String,
    #[arg(long, default_value = "auto")]
    pub captcha_mode: String,
    #[arg(long, default_value = "chrome")]
    pub fingerprint: String,
    #[arg(long, default_value = "")]
    pub client_ids: String,
    #[arg(long, default_value = "audio")]
    pub obfs: String,
    #[arg(long, default_value = "udp")]
    pub turn_transport: String,
    #[arg(long = "gen", default_value_t = 0)]
    pub generation: u64,
    #[arg(long, default_value = "")]
    pub salt: String,
    #[arg(long, default_value = "")]
    pub tun_uds: String,
    #[arg(long, default_value_t = false)]
    pub validate_vk_hashes: bool,
}

impl CArguments {
    fn to_args_vec(&self) -> Vec<String> {
        let mut args = Vec::new();
        if !self.turn.is_empty() {
            args.push(format!("--turn={}", self.turn));
        }
        if !self.port.is_empty() {
            args.push(format!("--port={}", self.port));
        }
        if !self.listen.is_empty() {
            args.push(format!("--listen={}", self.listen));
        }
        if !self.vk.is_empty() {
            args.push(format!("--vk={}", self.vk));
        }
        if !self.vk_hash_mode.is_empty() {
            args.push(format!("--vk-hash-mode={}", self.vk_hash_mode));
        }
        if !self.peer.is_empty() {
            args.push(format!("--peer={}", self.peer));
        }
        args.push(format!("--workers={}", self.workers));
        if self.allow_hash_redistribution {
            args.push("--allow-hash-redistribution".to_string());
        }
        if !self.device_id.is_empty() {
            args.push(format!("--device-id={}", self.device_id));
        }
        if !self.password.is_empty() {
            args.push(format!("--password={}", self.password));
        }
        if !self.vk_auth_mode.is_empty() {
            args.push(format!("--vk-auth-mode={}", self.vk_auth_mode));
        }
        if !self.captcha_mode.is_empty() {
            args.push(format!("--captcha-mode={}", self.captcha_mode));
        }
        if !self.fingerprint.is_empty() {
            args.push(format!("--fingerprint={}", self.fingerprint));
        }
        if !self.client_ids.is_empty() {
            args.push(format!("--client-ids={}", self.client_ids));
        }
        if !self.obfs.is_empty() {
            args.push(format!("--obfs={}", self.obfs));
        }
        if !self.turn_transport.is_empty() {
            args.push(format!("--turn-transport={}", self.turn_transport));
        }
        args.push(format!("--gen={}", self.generation));
        if !self.salt.is_empty() {
            args.push(format!("--salt={}", self.salt));
        }
        if !self.tun_uds.is_empty() {
            args.push(format!("--tun-uds={}", self.tun_uds));
        }
        if self.validate_vk_hashes {
            args.push("--validate-vk-hashes".to_string());
        }
        args
    }
}

// ===== C-ABI экспорт =====

/// Запуск клиента с параметрами (как в бинарнике)
#[no_mangle]
pub extern "C" fn run_client(
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
    let args = CArguments {
        peer: unsafe { cstr_to_string(peer) },
        vk: unsafe { cstr_to_string(vk) },
        password: unsafe { cstr_to_string(password) },
        listen: unsafe { cstr_to_string(listen) },
        workers: workers as usize,
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

    let args_vec = args.to_args_vec();
    push_log(format!("[LIB] Запуск с параметрами: {:?}", args_vec));

    // Создаём рантайм с ограниченным числом потоков (iOS-friendly)
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            push_log(format!("[LIB] Ошибка создания рантайма: {}", e));
            return -1;
        }
    };

    let result = rt.block_on(async {
        // Парсим аргументы через clap (как в бинарнике)
        let arg_strs: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
        let matches = match clap::Command::new("client")
            .disable_help_flag(true)
            .get_matches_from(arg_strs)
        {
            Ok(m) => m,
            Err(e) => {
                push_log(format!("[LIB] Ошибка парсинга: {}", e));
                return -2;
            }
        };

        // Собираем аргументы как в main.rs
        let arguments = main::Arguments::parse_from(arg_strs);
        match main::run(arguments).await {
            Ok(()) => 0,
            Err(e) => {
                push_log(format!("[LIB] Ошибка выполнения: {}", e));
                -3
            }
        }
    });

    result
}

/// Запуск с JSON-конфигом (альтернативный способ)
#[no_mangle]
pub extern "C" fn run_client_json(config_json: *const c_char) -> c_int {
    let json = unsafe { cstr_to_string(config_json) };
    push_log(format!("[LIB] Запуск с JSON: {}", json));

    let args: CArguments = match serde_json::from_str(&json) {
        Ok(a) => a,
        Err(e) => {
            push_log(format!("[LIB] Ошибка парсинга JSON: {}", e));
            return -1;
        }
    };

    run_client(
        args.peer.as_ptr() as *const c_char,
        args.vk.as_ptr() as *const c_char,
        args.password.as_ptr() as *const c_char,
        args.listen.as_ptr() as *const c_char,
        args.workers as c_int,
        args.device_id.as_ptr() as *const c_char,
        args.vk_hash_mode.as_ptr() as *const c_char,
        args.vk_auth_mode.as_ptr() as *const c_char,
        args.captcha_mode.as_ptr() as *const c_char,
        args.fingerprint.as_ptr() as *const c_char,
        args.client_ids.as_ptr() as *const c_char,
        args.obfs.as_ptr() as *const c_char,
        args.turn_transport.as_ptr() as *const c_char,
        args.generation,
        args.salt.as_ptr() as *const c_char,
        args.turn.as_ptr() as *const c_char,
        args.port.as_ptr() as *const c_char,
        args.allow_hash_redistribution,
        args.validate_vk_hashes,
        args.tun_uds.as_ptr() as *const c_char,
    )
}

/// Остановка клиента (graceful shutdown)
#[no_mangle]
pub extern "C" fn stop_client() -> c_int {
    push_log("[LIB] Остановка клиента");
    // TODO: реализовать graceful shutdown через CancellationToken
    0
}

/// Статус клиента (0 = stopped, 1 = running)
#[no_mangle]
pub extern "C" fn client_status() -> c_int {
    // TODO: реализовать статус
    0
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

// ===== main.rs для бинарника (игнорится при сборке .a) =====

#[cfg(not(target_os = "ios"))]
mod main {
    pub use crate::main_bin::*;
}

#[cfg(not(target_os = "ios"))]
#[path = "main.rs"]
mod main_bin;

#[cfg(target_os = "ios")]
mod main {
    // На iOS main не нужен — только lib.rs
}