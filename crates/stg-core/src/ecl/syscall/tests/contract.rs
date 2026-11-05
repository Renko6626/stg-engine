//! 号表、文档一致性与派发错误。

use super::*;

/// 冻结号表（号, 名, 期望族号）——逐条照 spec §4 的表；改动本表 = 改冻结面 = 过评审。
/// 提成助手供两条测试共用：结构判据（本表自身的性质）与白名单判据
/// （[`syscall_implemented`] 必须与本表**逐条等同**）。
fn frozen_table() -> &'static [(u16, &'static str, u16)] {
    &[
        (SYS_FRAME, "frame", 0),
        (SYS_PLAYER_X, "player_x", 0),
        (SYS_PLAYER_Y, "player_y", 0),
        (SYS_SELF_X, "self_x", 0),
        (SYS_SELF_Y, "self_y", 0),
        (SYS_SELF_VX, "self_vx", 0),
        (SYS_SELF_VY, "self_vy", 0),
        (SYS_SELF_SPEED, "self_speed", 0),
        (SYS_SELF_ANGLE, "self_angle", 0),
        (SYS_SELF_HP, "self_hp", 0),
        (SYS_SELF_HP_MAX, "self_hp_max", 0),
        (SYS_SELF_AGE, "self_age", 0),
        (SYS_SELF_ENEMY, "self_enemy", 0),
        (SYS_ENEMY_HP, "enemy_hp", 1),
        (SYS_ENEMY_X, "enemy_x", 1),
        (SYS_ENEMY_Y, "enemy_y", 1),
        (SYS_ENEMY_ALIVE, "enemy_alive", 1),
        (SYS_NEAREST_ENEMY, "nearest_enemy", 1),
        (SYS_AIM_PLAYER_ANGLE, "aim_player_angle", 1),
        (SYS_SPELL_TIMER, "spell_timer", 1),
        (SYS_SPELL_RESULT, "spell_result", 1),
        (SYS_ATAN2, "atan2", 1),
        (SYS_DIST, "dist", 1),
        (SYS_RAND_RANGE, "rand_range", 1),
        (SYS_SELF_INT, "self_int", 1),
        (SYS_SELF_FX, "self_fx", 1),
        (SYS_CREATE_BULLET, "create_bullet", 2),
        (SYS_CREATE_BULLET_PROGRAM, "create_bullet_program", 2),
        (SYS_CREATE_BULLETS_BATCH, "create_bullets_batch", 2),
        (SYS_SPAWN_ENEMY, "spawn_enemy", 2),
        (SYS_DROP_ITEM, "drop_item", 2),
        (SYS_SET_BULLET_SPEED, "set_bullet_speed", 3),
        (SYS_SET_BULLET_ANGLE, "set_bullet_angle", 3),
        (SYS_TURN_BULLET, "turn_bullet", 3),
        (SYS_SET_BULLET_VEL, "set_bullet_vel", 3),
        (SYS_SET_BULLET_ANG_VEL, "set_bullet_ang_vel", 3),
        (SYS_SET_BULLET_ACCEL, "set_bullet_accel", 3),
        (SYS_SET_BULLET_GRAVITY, "set_bullet_gravity", 3),
        (SYS_STOP_BULLET_FX, "stop_bullet_fx", 3),
        (SYS_AIM_BULLET_AT_PLAYER, "aim_bullet_at_player", 3),
        (SYS_MOVE_ENEMY_TO, "move_enemy_to", 4),
        (SYS_MOVE_VEL, "move_vel", 4),
        (SYS_MOVE_VEL_XY, "move_vel_xy", 4),
        (SYS_MOVE_ANGLE, "move_angle", 4),
        (SYS_MOVE_SPEED, "move_speed", 4),
        (SYS_SET_ANM_STATE, "set_anm_state", 4),
        (SYS_SET_INVULN, "set_invuln", 4),
        (SYS_SET_HITBOX, "set_hitbox", 4),
        (SYS_SET_HURTBOX, "set_hurtbox", 4),
        (SYS_SET_ENEMY_FLAG, "set_enemy_flag", 4),
        (SYS_SET_SELF_INT, "set_self_int", 4),
        (SYS_SET_SELF_FX, "set_self_fx", 4),
        (SYS_ADD_SCORE, "add_score", 5),
        (SYS_ADD_LIVES, "add_lives", 5),
        (SYS_ADD_BOMBS, "add_bombs", 5),
        (SYS_ADD_POWER, "add_power", 5),
        (SYS_DROP_CLEAR, "drop_clear", 5),
        (SYS_DROP_ADD, "drop_add", 5),
        (SYS_DROP_ITEMS, "drop_items", 5),
        (SYS_DIE, "die", 5),
        (SYS_KILL_ALL_ENEMIES, "kill_all_enemies", 5),
        (SYS_CLEAR_BULLETS, "clear_bullets", 5),
        (SYS_CLEAR_BULLETS_AT, "clear_bullets_at", 5),
        (SYS_BGM, "bgm", 5),
        (SYS_BG, "bg", 5),
        (SYS_BG_PHASE, "bg_phase", 5),
        (SYS_TIME_STOP_PLAYER, "time_stop_player", 5),
        (SYS_SET_PLAYER_HITBOX, "set_player_hitbox", 5),
        (SYS_SH_RESET, "sh_reset", 6),
        (SYS_SH_SPRITE, "sh_sprite", 6),
        (SYS_SH_OFFSET, "sh_offset", 6),
        (SYS_SH_OFFSET_ABS, "sh_offset_abs", 6),
        (SYS_SH_OFFSET_RAD, "sh_offset_rad", 6),
        (SYS_SH_DIST, "sh_dist", 6),
        (SYS_SH_ANGLE, "sh_angle", 6),
        (SYS_SH_SPEED, "sh_speed", 6),
        (SYS_SH_COUNT, "sh_count", 6),
        (SYS_SH_AIM, "sh_aim", 6),
        (SYS_SH_RING, "sh_ring", 6),
        (SYS_SH_XFORM, "sh_xform", 6),
        (SYS_SH_XFORM_PROGRAM, "sh_xform_program", 6),
        (SYS_SH_TASK, "sh_task", 6),
        (SYS_SH_REQ, "sh_req", 6),
        (SYS_SH_FIRE, "sh_fire", 6),
        (SYS_ACCUM_VALUE, "accum_value", 1),
        (SYS_ACCEL_VALUE, "accel_value", 1),
        (SYS_HERMITE_VALUE, "hermite_value", 1),
        (SYS_ACCUM_START, "accum_start", 7),
        (SYS_ACCEL_START, "accel_start", 7),
        (SYS_HERMITE_START, "hermite_start", 7),
        (SYS_GET_VAR, "get_var", 7),
        (SYS_SET_VAR, "set_var", 7),
        (SYS_PULSE_SIGNAL, "pulse_signal", 7),
        (SYS_EMIT_REQ, "emit_req", 7),
        (SYS_FX_AT, "fx_at", 7),
        (SYS_FX_ON, "fx_on", 7),
        (SYS_STAGE_CLEAR, "stage_clear", 7),
        (SYS_BOSS_SET, "boss_set", 7),
        (SYS_SPELL_BEGIN, "spell_begin", 7),
        (SYS_SPELL_END, "spell_end", 7),
        (SYS_LASER_CREATE, "laser", 8),
        (SYS_LASER_SPEED, "lz_speed", 8),
        (SYS_LASER_START, "lz_start", 8),
        (SYS_LASER_OMEGA, "lz_omega", 8),
        (SYS_LASER_ROTATE, "lz_rotate", 8),
        (SYS_LASER_AIM, "lz_aim", 8),
        (SYS_LASER_ANCHOR, "lz_anchor", 8),
        (SYS_LASER_ORIGIN, "lz_origin", 8),
        (SYS_LASER_CANCEL, "lz_cancel", 8),
        (SYS_LASER_ALIVE, "lz_alive", 8),
        (SYS_LASER_X, "lz_x", 8),
        (SYS_LASER_Y, "lz_y", 8),
        (SYS_LASER_ANGLE, "lz_angle", 8),
        (SYS_LASER_NEAR, "lz_near", 8),
        (SYS_LASER_FAR, "lz_far", 8),
    ]
}

/// 【本刀的主判据】号表族结构：103 条、无重号、每条落在其声明族的百位区间内
/// （原 74 条 + 自机能力刀 `513`/`560` = 76；表现契约 v2 再加 `430 set_anm_state`/`721 fx_at`/`722 fx_on` = 79；壳子刀加 `723 stage_clear` = 80；
/// 激光池刀再加 8xx 十条 = 97；激光读口五条 810–814 = 102；自机判定写口 561 = 103）。
///
/// 这一刀是大规模机械重排，判别力要求与常规刀不同——不是"新行为对不对"，而是
/// "**有没有搬错、搬漏、搬重**"。故判据是号表自身的结构性质，不是某条 syscall 的行为。
///
/// # 判别力边界（终审实测，2026-07-31——别高估这条测试）
///
/// **抓不到「族内互换」。** [`frozen_table`] 是**符号引用**常量的（`(SYS_ATAN2, "atan2",
/// 1)`），所以把 `SYS_ATAN2` 与 `SYS_DIST` 的值对调（140 ↔ 141）之后，本测试的三条
/// 断言**全部照过**：仍是 74 条、仍两两不等、仍都落在 `1xx`。实测把这对值对调后
/// **全仓 947 条测试无一转红**——这不是覆盖漏洞，因为号本身是任意的，族内换值在行为上
/// 确实是 no-op（调用方一律符号引用，`ecl-meta.json` 也不含号）。但上面那句"有没有
/// 搬错"要按字面读会读出过强的承诺：**「值错、族对」这一格本测试是瞎的**。
/// 那一格**全仓唯一**的网是 [`ecl_ops_doc_syscall_numbers_match_the_constants`]——
/// 它拿 `docs/ecl-ops.md`（号表的权威呈现面）里的**字面数字**双向对常量，
/// 别把它当成只防文档漂移的东西而删掉。
///
/// **「搬重」比本测试更早被抓住——是编译期错误。** 把 `SYS_DIST` 改成与 `SYS_ATAN2`
/// 同值，rustc 在**两处**报 `unreachable pattern`：[`syscall_implemented`] 的白名单
/// `matches!` 与 [`dispatch`] 的 `match`（两处都是 `const` 模式匹配）。CI 的
/// `-D warnings` 下这是硬失败，压根编译不过来跑测试。故 (b) 那条唯一性断言实际是
/// **第二道网**，价值在于把这条性质写成可读的意图，而不是它先发现问题。
#[test]
fn syscall_table_is_hundred_partitioned_and_unique() {
    let table = frozen_table();

    assert_eq!(
        table.len(),
        115,
        "74 + 自机能力刀两条（513/560）+ 表现契约 v2 三条（430/721/722）+ 壳子刀 723 − 玩法刀退役 513 \
         + boss 换段刀八条（026/131/440/441/442/443/531/541）= 87；激光池刀 8xx 十条 = 97；激光读口 810–814 = 102；自机判定写口 561 = 103；显式插值槽六条 = 109；敌人共享槽四条 = 113；xform 程序入口两条 = 115：增改需同步这个数"
    );

    // (a) 族归属：搬错族立刻红
    for &(num, name, fam) in table {
        assert_eq!(
            num / 100,
            fam,
            "{name}(={num}) 应落在 {fam}xx 族，实得 {}xx",
            num / 100
        );
    }

    // (b) 两两不等：搬重立刻红（O(n²) 但 n=76，测试里无所谓）
    for (i, &(a, na, _)) in table.iter().enumerate() {
        for &(b, nb, _) in &table[i + 1..] {
            assert_ne!(a, b, "{na} 与 {nb} 撞号（都是 {a}）");
        }
    }

    // (c) 与 op 号空间错开：op 是 u8、现最大 60（OP_SPAWN_PATTERN）。这条只对**非
    //     0xx 族**成立——1xx..7xx 全部 >= 100，与 op 号空间永久不相交；0xx 族的号
    //     本来就 < 100（000-032），与 op 号空间的重叠是既有状态，本刀不承诺解决
    //     （spec §3 "syscall 全部推到 100 以上就与 op 号错开"这句对 0xx 族不成立，
    //     已在设计评审收窄，见 task-1-report.md）。
    for &(num, name, fam) in table {
        if fam != 0 {
            assert!(
                num >= 100,
                "{name}(={num}) 应与 op 号空间(u8,最大 60)错开，但仍 <100",
            );
        }
    }
}

// ── `docs/ecl-ops.md` 号表 ↔ 常量的双向钉死（终审补，2026-07-31）────────────────

/// `"000"` / `"140"` → `Some(0)` / `Some(140)`；其余（`---`、`300-330`、正文词）→ `None`。
/// **只认三位补零形式**，这是号表列的排版约定（见 `ecl-ops.md` 0xx 族那条排版说明）。
fn doc_num(tok: &str) -> Option<u16> {
    let t = tok.trim();
    (t.len() == 3 && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
}

/// 依次取出 cell 里所有反引号包起来的**原样** token（不判字符集，`$` 前缀剥掉）。
fn backticked(cell: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = cell;
    while let Some(i) = rest.find('`') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('`') else { break };
        out.push(after[..j].trim_start_matches('$'));
        rest = &after[j + 1..];
    }
    out
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// 从 `docs/ecl-ops.md` 的 `## syscall 号表` 一节抽出全部 `(号, 名)` 对。
///
/// 两个来源，都是文档**自己既有的**排版，不是为测试新加的标记：
/// - **表行** `| 号 | 名 | 参数 | 返回 |`：首格是号（`NNN` 或 `NNN/NNN`），
///   次格开头是反引号名。`| 010/011 | `player_x/y` |` 这种合并写法按 `_` 词干展开。
/// - **围栏块**里相邻的 `NNN name`（3xx 弹 setter 九连只在围栏里逐条列名，
///   表行是聚合的 `| 300-330 | 弹 setter 族 |`）。
///
/// 第二个返回值 = 那些**没能配上名**的格子里的三位数（如 `300-330` 的两个端点），
/// 只做"必须是真号"的弱检查。
fn parse_doc_number_table() -> (Vec<(u16, String)>, Vec<u16>) {
    const DOC: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/ecl-ops.md"
    ));

    let start = DOC
        .find("\n## syscall 号表")
        .expect("ecl-ops.md 缺 `## syscall 号表` 节标题——文档结构变了，本测试要跟着改");
    let sec = &DOC[start + 1..];
    let end = sec[3..].find("\n## ").map_or(sec.len(), |i| i + 4);
    let sec = &sec[..end];

    let mut pairs: Vec<(u16, String)> = Vec::new();
    let mut loose: Vec<u16> = Vec::new();
    let mut in_fence = false;

    for line in sec.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            // 围栏：相邻两 token 形如 `NNN name`
            let toks: Vec<&str> = line.split_whitespace().collect();
            for w in toks.windows(2) {
                if let (Some(n), true) = (doc_num(w[0]), is_ident(w[1])) {
                    pairs.push((n, w[1].to_string()));
                }
            }
            continue;
        }
        let Some(body) = line.strip_prefix('|') else {
            continue;
        };
        let mut cells = body.split('|');
        let (Some(num_cell), Some(name_cell)) = (cells.next(), cells.next()) else {
            continue;
        };
        let nums: Vec<Option<u16>> = num_cell.trim().split('/').map(doc_num).collect();
        if nums.iter().any(Option::is_none) {
            // 聚合格（`300-330`）或表头分隔行（`---`）：只捞里面的三位数弱检查
            let bytes = num_cell.as_bytes();
            for i in 0..bytes.len() {
                if bytes[i].is_ascii_digit()
                    && (i == 0 || !bytes[i - 1].is_ascii_digit())
                    && let Some(n) = doc_num(&num_cell[i..(i + 3).min(num_cell.len())])
                {
                    loose.push(n);
                }
            }
            continue;
        }
        let nums: Vec<u16> = nums.into_iter().flatten().collect();
        let ticked = backticked(name_cell);
        let mut names: Vec<String> = ticked
            .iter()
            .filter(|s| is_ident(s))
            .map(|s| s.to_string())
            .collect();
        // 合并写法 `player_x/y` → player_x / player_y
        if names.len() < nums.len()
            && let Some((a, b)) = ticked.first().and_then(|t| t.split_once('/'))
            && let Some((stem, _)) = a.rsplit_once('_')
        {
            names = vec![a.to_string(), format!("{stem}_{b}")];
        }
        assert!(
            names.len() >= nums.len(),
            "ecl-ops.md 号表行 `{}` 的号有 {} 个、认得出的反引号名只有 {} 个——\
             排版变了就得改 `parse_doc_number_table`（别把这条测试关掉）",
            line.chars().take(60).collect::<String>(),
            nums.len(),
            names.len(),
        );
        for (n, name) in nums.into_iter().zip(names) {
            pairs.push((n, name));
        }
    }
    (pairs, loose)
}

/// **`docs/ecl-ops.md` 的号表必须与 `SYS_*` 常量两向一致**（终审补，2026-07-31）。
///
/// 补的是一个**沉默漂移面**：`ecl-ops.md` 是号表层的权威呈现面（M4 对端与回放头要认的
/// 冻结契约），而在此之前**仓内没有任何东西把它和常量绑住**——一次手滑编辑就能让文档
/// 与代码分家，且不会有任何测试转红。号表重排那一刀是靠**手工写脚本**比对才确认干净的，
/// 那种一次性验证保不住下一次。
///
/// 两个方向都断言（缺一个就只是半张网）：
/// - **文档 → 常量**：文档里出现的每个 `(号, 名)` 对都得在 [`frozen_table`] 里；
/// - **常量 → 文档**：102 条常量每条都得在文档里出现，**漏记一条即红**。
///
/// **它还有第二重职责，别只当它是"防文档漂移"**：本条是**全仓唯一**能抓到
/// **族内互换**（号换了、族没换，如 `SYS_ATAN2` ↔ `SYS_DIST`）的测试。
/// [`syscall_table_is_hundred_partitioned_and_unique`] 那张表**符号引用**常量，对具体
/// 取值免疫；实测把这一对的值对调后，**全仓 947 条测试无一转红**（终审复跑，2026-07-31）。
/// 本条按**字面数字**比对，是那一格唯一的网——变异实证见下（对调文档号列即红，
/// 而结构测试照绿）。删本条 = 把那一格重新变瞎。
///
/// 解析靠的是文档自己既有的排版（号列 + 反引号名），没有新加机器可读标记。排版真变了
/// 本测试会**带着行内容响亮失败**并要求同步改解析器——号表是冻结面，这个代价是对的。
#[test]
fn ecl_ops_doc_syscall_numbers_match_the_constants() {
    let table = frozen_table();
    let (doc_pairs, loose) = parse_doc_number_table();

    assert!(
        doc_pairs.len() >= 79,
        "只从 ecl-ops.md 解析出 {} 个 (号,名) 对，远少于 79——解析器多半没跟上排版变更",
        doc_pairs.len()
    );

    // 方向一：文档有而常量无（含"号对名错"与"名对号错"）
    for (num, name) in &doc_pairs {
        assert!(
            table.iter().any(|&(n, nm, _)| n == *num && nm == name),
            "ecl-ops.md 记着 `{num} = {name}`，但 `SYS_*` 常量里没有这一对；\
             号表以 `ecl::syscall` 的常量为唯一权威，改文档别改代码"
        );
    }

    // 方向二：常量有而文档无（漏记一条）
    for &(num, name, _) in table {
        assert!(
            doc_pairs.iter().any(|(n, nm)| *n == num && nm == name),
            "`SYS_{}` = {num} 在 `docs/ecl-ops.md` 的号表里查无此条——新增/改号必须同步文档",
            name.to_uppercase()
        );
    }

    // 聚合格（如 `| 300-330 | 弹 setter 族 |`）的端点也得是真号
    for n in loose {
        assert!(
            syscall_implemented(n),
            "ecl-ops.md 号表的聚合格里出现了 {n}，但它不是任何 syscall 的号"
        );
    }
}

/// 坏 syscall 号（不在 v1 号表内）→ Fault（`dispatch` 默认臂，复用 `FAULT_BAD_OP`）。
#[test]
fn dispatch_bad_syscall_number_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert_eq!(call(&mut w, &ecl, &mut task, 9999, &[]), Err(FAULT_BAD_OP));
}

/// [`syscall_implemented`] 的白名单必须与冻结号表**逐条等同**——它是跨 crate 的唯一
/// 出口（`dispatch` 是 `pub(crate)`、[`frozen_table`] 是 `cfg(test)`，编译器 crate
/// 两个都够不着），一旦与真实派发面漂移，`stg-ecl-compiler` 那条 `is_op` 一致性测试
/// 就会拿错判据、变成假绿。
///
/// 判据是**全 `u16` 域扫一遍**（65536 次 `matches!`，测试里无所谓），不是"照表逐条问
/// 一遍"——后者只能抓"漏了一条"，抓不到"多写了一条表里没有的号"。两个方向都要：
/// 加 syscall 忘了加进白名单 → 数量对不上；白名单里留了一条已删的号 → 同样红。
#[test]
fn syscall_whitelist_matches_the_frozen_table() {
    let table = frozen_table();

    // 方向一：表里每条都必须在白名单内。
    for &(num, name, _) in table {
        assert!(
            syscall_implemented(num),
            "{name}(={num}) 在冻结号表里却不在 syscall_implemented 白名单内"
        );
    }

    // 方向二：全域里为真的号，恰好只有表里那些（抓"多写了一条"）。
    let live: Vec<u16> = (0..=u16::MAX).filter(|&n| syscall_implemented(n)).collect();
    assert_eq!(
        live.len(),
        table.len(),
        "白名单为真的号有 {} 个，冻结号表却是 {} 条",
        live.len(),
        table.len()
    );
    for n in &live {
        assert!(
            table.iter().any(|&(num, _, _)| num == *n),
            "白名单里的 {n} 不在冻结号表内（多写/该删未删）"
        );
    }

    // 方向三：把白名单钉到 `dispatch` 的兜底臂上——白名单说没有的号，`dispatch`
    // 必须以 `FAULT_BAD_OP` 拒绝（抽查，不做全域派发：派发有副作用）。
    let (mut w, ecl) = fresh();
    for probe in [9999u16, 33, 99, 104, 199, 899, u16::MAX] {
        assert!(!syscall_implemented(probe), "{probe} 不该在白名单里");
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, probe, &[]),
            Err(FAULT_BAD_OP),
            "白名单外的 {probe} 应被 dispatch 兜底臂拒绝"
        );
    }
}

/// 栈下溢（参数不足）→ `Fault(FAULT_STACK)`：`SET_VAR` 需 2 参，空栈直接派发。
#[test]
fn dispatch_stack_underflow_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert_eq!(
        call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[]),
        Err(FAULT_STACK)
    );
}
