@echo off
pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0sentrux-shim.ps1" %*
