# 显式读取的状态插值槽

用户已确认：三个独立操作与变量槽位，显式读取，不自动回写变量。

每任务每类4槽，fx值；同步调用共享任务槽，spawn新任务清零。启动立即读初值，下一有效任务帧开始推进；wait期间推进，冻结期间不推进。累加/加速恰好更新N次并保持值；Hermite到期精确保持终点，切线按归一化时间定义。零时长：累加/加速保持初值，Hermite立即终点。重启覆盖原槽。坏槽/负或超过65535的时长 no-op+诊断，坏读取返回0。

新增6个syscall及同名builtin：accum_start/value、accel_start/value、hermite_start/value。状态放TaskPool并行数组，进checksum/save/copy，spawn完整重置。ENGINE_VER 25→26，旧存档需拒绝。无变量引用、自动写回、新运动接口或导入器扩展。

- [x] 真实ECL行为测试：启动/逐帧/wait/边界/槽隔离/快照续算。
- [x] POD状态、任务调度推进、syscall、builtin、常量和版本。
- [x] 生成元数据，更新接口/原作对照文档。
- [x] 相关测试、fmt/clippy、golden和恢复验证，独立审阅。

验证结果（2026-10-23）：

- `cargo test --offline -p stg-core -p stg-ecl-compiler -p stg-harness`：core 805通过；compiler穷举名单遗漏6条，随后补齐。
- `cargo test --offline -p stg-ecl-compiler -p stg-harness`：compiler337、插值集成3、reseed集成1、harness54及doc-test通过。
- `cargo clippy --offline -p stg-core -p stg-ecl-compiler -p stg-harness --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check`通过。
- `cargo run --offline -p stg-harness -- golden --out /tmp/stg-interp26-golden.txt`：三段成功；SHA256 `75d6a0e3dbff13e7ccb736f5240142ca0407a293d61cd8adc87cc129bd50d706`，新增哈希状态使旧checksum变化。
- `cargo run --offline -p stg-harness -- verify-tables`：全部表通过；`gen-ecl-meta`已同步两处生成输出。
- 独立只读审阅review_easing30未发现正确性问题；未重复跑集中测试。
- 行为覆盖：wait推进、初值和保持、三类独立槽、旧速度优先、Hermite非零切线及终点、零时长、覆盖重启、坏参no-op+4次计数、同步共享／异步隔离、槽复用清零、活跃槽save/load与copy后续算checksum一致。

未升级训练wheel／卡池，未做原作动态／跨平台对拍，未提交或推送。源计时与自动变量回写的转换仍待导入器实施。

## 本地提交前复核（2026-10-26）

- `cargo test --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl`：core 805、compiler 337及4项集成测试通过；harness 52项通过，另2项本地端口测试被沙箱拒绝。获准后在沙箱外运行 `cargo test --offline -q -p stg-harness viewer::tests`，相关6项全部通过。
- `cargo test --offline -q -p stg-rl`：49项通过。
- `cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings`、`cargo check --offline --manifest-path crates/stg-py/Cargo.toml`通过。
- `crates/stg-py/.venv/bin/python -m pytest -q -p no:cacheprovider crates/stg-py/tests/test_smoke.py -k current_start_indices`：1项通过，使用已有0.4.1／ENGINE_VER24 wheel，仅验证起点查询；未重建ENGINE_VER26 wheel。
- `cargo run --offline -q -p stg-harness -- verify-tables`通过；`cargo run --offline -q -p stg-harness -- golden --out /tmp/stg-engine-precommit-20261026-golden.txt`三段成功，SHA256与上述26版记录一致。
- 本批纳入本地提交，未推送；原作动态和跨平台对拍仍未执行。上文“未提交”保留为2026-10-23当时记录。
