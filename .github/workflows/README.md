# GitHub Actions Cross-Platform Build Workflow

Этот workflow автоматически собирает ваш Rust-проект под **Windows**, **Linux** и **Android** при каждом push/pull request.

## 🔧 Что собирается

| Платформа | Архитектура | Формат | Target |
|-----------|-------------|--------|--------|
| **Windows** | x86_64 | .exe, .dll | `x86_64-pc-windows-msvc` |
| **Linux** | x86_64 | .so, .a, client | `x86_64-unknown-linux-gnu` |
| **Linux** | i686 | .so, .a, client | `i686-unknown-linux-gnu` |
| **Android** | arm64-v8a | .so | `aarch64-linux-android` |
| **Android** | armeabi-v7a | .so | `armv7-linux-androideabi` |

## 📦 Артефакты

Каждый job загружает свои артефакты:
- `windows-x86_64` — .exe, .dll, .pdb
- `linux-x86_64-unknown-linux-gnu` — .so, .a, client
- `linux-i686-unknown-linux-gnu` — .so, .a, client
- `android-arm64-v8a` — .so
- `android-armeabi-v7a` — .so
- `all-platforms-{sha}` — единый архив со всем сборками

## 🚀 Кеширование

- **Cargo**: кеширует `~/.cargo` и `target/` для быстрой пересборки
- **Android SDK/NDK**: кеширует ~2GB SDK для ускорения сборки Android

## 🛠️ Настройка

1. Убедитесь, что ваш проект имеет структуру:
```

project/
├── Cargo.toml
├── Cargo.lock
├── src/
│   └── lib.rs   # или main.rs
├── app/         # (опционально) для Android JNI
│   └── src/main/jniLibs/
└── .github/workflows/build.yml

```

2. Если ваш бинарный файл называется иначе, чем `client`, измените пути в секциях `Upload artifacts`.

3. Для Android-сборки укажите правильную версию NDK (сейчас 26.3.11579264).

## 📝 Проверки перед сборкой

Workflow сначала запускает:
- `cargo fmt --check`
- `cargo clippy` с `-D warnings`
- `cargo test`

Если любая проверка падает — сборка не запускается.

## 🔄 Запуск вручную

Можно запустить вручную через GitHub UI:  
`Actions` → `Cross-Platform Build` → `Run workflow`

## 🧪 Тестирование локально

Для локальной проверки Android-сборки:

```bash
# Установка cargo-ndk
cargo install cargo-ndk

# Сборка arm64
cargo ndk --target aarch64-linux-android --platform 33 --build-mode release

# Сборка armv7
cargo ndk --target armv7-linux-androideabi --platform 33 --build-mode release
```

## 📄 Лицензия

SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
Copyright © 2026 amurcanov

