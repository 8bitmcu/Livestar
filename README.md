# Livestar

Livestar is a minimal, configurable starfield Live Wallpaper for Android devices.

***

Livestar utilizes a dual-language architecture that pairs Java for Android system integration and UI management with Rust for high-performance rendering. The system operates by bridging the Android `WallpaperService` lifecycle with a native Rust renderer. While the Android application layer handles user configuration and surface management, the Rust layer manages the heavy lifting; specifically starfield generation and GPU command encoding via the `wgpu` graphics library. The core rendering logic is implemented in Rust to ensure maximum efficiency and battery life.

![assets/preview.gif](assets/preview.gif)
