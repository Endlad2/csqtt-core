# csqtt-ios-core

iOS static library core for CSQTT, synced with `rust-client` **2.5.1**.

## What changed in 2.5.1 (vs 2.5.0)

This is a **sync release**: `rust-client/` is already 2.5.1; the iOS core was
still pinned to the 2.5.0 C-ABI and CLI surface. 2.5.1 closes that gap.

### C-ABI: `csqtt_run` — 30 → **29** arguments

Removed (не существуют как CLI-флаги в `rust-client/main.rs` 2.5.1):

- `device_id: *const c_char` — был мёртвым полем в 2.5.0.
- `allow_hash_redistribution: bool` — тоже мёртвый флаг.

> **Swift / Kotlin обязаны обновить сигнатуру `csqtt_run`.** Порядок оставшихся
> 29 аргументов сохранён без перестановок, чтобы правки были минимальными.

### New C-ABI function: `csqtt_validate_vk_hashes`

```c
typedef void (*csqtt_hash_check_callback_t)(
    const char *hash, int32_t status, int64_t code, const char *message);

int32_t csqtt_validate_vk_hashes(
    const char *vk,
    const char *fingerprint,
    const char *client_ids,
    csqtt_hash_check_callback_t callback);
```

`status`:

| value ↕▾ | meaning ↕▾ |
|---|---|
| −`0` | valid |
| −`1` | invalid |
| −`2` | unavailable |
⚙

Возвращает `0` при успехе; `-1` — runtime не создан; `-2` — нет хешей;
`-3` — колбэк NULL.  Колбэк вызывается синхронно на каждый хеш в порядке
входной строки. `message` всегда валидная NUL-terminated строка (может быть
пустой).

Это переиспользует `auth::check_vk_hashes` из `rust-client`, так что поведение
совпадает с CLI-путём `--validate-vk-hashes` в `main.rs` 2.5.1.

### Auto-JS выпилен из iOS-ядра

`vk_hash_mode=auto_js` и `vk_auth_mode=auto_js` **не поддерживаются** в
статической библиотеке и приводят к ошибке из `csqtt_run` (`-3`). Причина:
Auto-JS требует stdin-канал для `VK_JS_BOOTSTRAP` и `CredentialBroker`,
которых в static lib нет.

Допустимые режимы:

- `vk_hash_mode`: `manual` (или пусто)
- `vk_auth_mode`: `vkcalls`, `legacy`

Если нужен Auto-JS — он реализуется на стороне app (Swift/Kotlin) и передаётся
в iOS-ядро как обычные `manual`-хеши.

### `--domains-file` не пробрасывается

В iOS-ядре нет понятия внешних `--domains-file`. Шаблонный список доменов
(`template_domains=true`) встроен в бинарник через `include_str!("../domains.txt")`
из `rust-client/sni.rs`, поэтому файловый I/O на устройстве не нужен. Если
приложению нужны собственные домены — передайте их через `direct_domains`
(comma/space/semicolon-separated).

### Runtime threads

В iOS-ядре нет флага `--runtime-threads`. Треды runtime выбираются так же, как
в `main.rs::runtime_worker_threads_with_limit(workers, 0, available)`, но
`available` фиксирован в `IOS_DEFAULT_RUNTIME_THREADS = 4` (перф-ядра iOS):

```
runtime_threads = ceil(normalize_worker_count(workers) / 8).clamp(1, 4)
```

### Events

`Events::new(false)` — env-переменных `CSQTT_EVENTS` в iOS нет, поэтому события
выключены. `push_log` / `csqtt_set_log_callback` — основной канал логов.

## C-ABI: `csqtt_run`

```
peer, vk, password, listen, workers,
vk_hash_mode, vk_auth_mode, captcha_mode, fingerprint, client_ids,
obfs, turn_transport, srtp_mode, generation, salt,
turn, custom_turn_ips, port, validate_vk_hashes, tun_uds,
domain_routing, exclude_ru_zones, template_domains, domain_whitelist, direct_domains,
ip_routing, ip_whitelist, direct_ips, exact_workers
```

Все `*const c_char` толерантны к NULL (трактуются как пустая строка).

### Kotlin example

```
external fun csqtt_run(
    peer: String?, vk: String?, password: String?, listen: String?, workers: Int,
    vkHashMode: String?, vkAuthMode: String?, captchaMode: String?, fingerprint: String?,
    clientIds: String?, obfs: String?, turnTransport: String?, srtpMode: String?,
    generation: Long, salt: String?, turn: String?, customTurnIps: String?, port: String?,
    validateVkHashes: Boolean, tunUds: String?,
    domainRouting: Boolean, excludeRuZones: Boolean, templateDomains: Boolean,
    domainWhitelist: Boolean, directDomains: String?,
    ipRouting: Boolean, ipWhitelist: Boolean, directIps: String?, exactWorkers: Boolean,
): Int
```

### Swift example

```
@_silgen_name("csqtt_run")
func csqtt_run(
    _ peer: UnsafePointer<CChar>?, _ vk: UnsafePointer<CChar>?, _ password: UnsafePointer<CChar>?,
    _ listen: UnsafePointer<CChar>?, _ workers: Int32,
    _ vkHashMode: UnsafePointer<CChar>?, _ vkAuthMode: UnsafePointer<CChar>?,
    _ captchaMode: UnsafePointer<CChar>?, _ fingerprint: UnsafePointer<CChar>?,
    _ clientIds: UnsafePointer<CChar>?, _ obfs: UnsafePointer<CChar>?,
    _ turnTransport: UnsafePointer<CChar>?, _ srtpMode: UnsafePointer<CChar>?,
    _ generation: UInt64, _ salt: UnsafePointer<CChar>?,
    _ turn: UnsafePointer<CChar>?, _ customTurnIps: UnsafePointer<CChar>?,
    _ port: UnsafePointer<CChar>?, _ validateVkHashes: Bool, _ tunUds: UnsafePointer<CChar>?,
    _ domainRouting: Bool, _ excludeRuZones: Bool, _ templateDomains: Bool,
    _ domainWhitelist: Bool, _ directDomains: UnsafePointer<CChar>?,
    _ ipRouting: Bool, _ ipWhitelist: Bool, _ directIps: UnsafePointer<CChar>?,
    _ exactWorkers: Bool
) -> Int32
```

## Build

```
cargo build --release --target aarch64-apple-ios --profile ios-release
```

The output `libcsqtt_ios_core.a` can be linked into the iOS app.

## Notes

- `run_client` больше не проверяет inline `validate_vk_hashes` — используйте
отдельную функцию `csqtt_validate_vk_hashes`, которая синхронно вызывает
колбэк на каждый хеш. Поведение идентично CLI-пути `--validate-vk-hashes`.
- `ready_credential_tx` всегда `None` (Auto-JS не используется).
- `tun_uds` работает как в 2.5.0: абстрактный UDS, куда Android/iOS отдают
TUN FD через `SCM_RIGHTS`.
- `domains.txt` встроен в бинарник через `include_str!` в `sni.rs`.

