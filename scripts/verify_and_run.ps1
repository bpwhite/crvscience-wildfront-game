$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

Write-Host "=== [1/4] Running Full Workspace Test Suite (wf_core, wf_assets, wf_sim, wf_client) ===" -ForegroundColor Cyan
cargo test --workspace

Write-Host "=== [2/4] Verifying Asset & Voxel Brickmap Compiler (152 Cards + 170 Voxel Models) ===" -ForegroundColor Cyan
cargo run -p wf_assets --release

Write-Host "=== [3/4] Running Deterministic 2-Ply Headless AI Smoke Match + Replay Hash Check ===" -ForegroundColor Cyan
cargo run -p wf_sim --release -- match --p1-origin Australia --p1-primary Mammal --p1-secondary Mammal --p2-origin Africa --p2-primary Fish --p2-secondary Fish --scenario 4 --ai grandmaster --save-replay replays/smoke_test.ron
cargo run -p wf_sim --release -- verify-replay replays/smoke_test.ron

Write-Host "=== [4/4] Launching Wildfront Battlegrounds v3.4.1 Native Voxel Client ===" -ForegroundColor Cyan
cargo run -p wf_client --release
