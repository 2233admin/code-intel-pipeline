@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0sentrux-shim.ps1" %*
