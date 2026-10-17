@echo off
cd /d "%~dp0"
cargo run --release -p vidlib-tauri
if errorlevel 1 pause
