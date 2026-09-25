# 9 · 激光

> 这一篇讲直线激光：`laser()` 建一条，`lz_*` 族操作它。激光是**一等实体**——射线原点 +
> 方向 + 射线上 `[start, end]` 一段，与弹走同一套池/句柄纪律。读之前先读
> [4 · 弹](4-bullets.md) 的发射器节：激光的"配一遍、改一句"思路和它很像，只是没有发射器槽。

**这一篇的三种形态**（也是转写原作时最常遇到的三种）：

| 形态 | 参数怎么填 | 例子 |
|---|---|---|
| 预警线 → 生效扫射 | `warn > 0`、`speed = 0`，出生后 `lz_omega` | 竖一条线警告 30 帧，然后慢慢扫 |
| 自机狙 | `warn > 0`，出生后 `lz_aim` | 线摆到自机方向再射 |
| 飞出去的棒子 | `warn = 0`、`active` 很大，出生后 `lz_speed` | 192px 的短棒沿射线匀速飞出 |

## 建一条激光

```
laser(color, x, y, angle, len, width, warn, active, fade) -> int
```

- `color ∈ 0..=15`：激光只有一种截面贴图，颜色就是全部外观。越界在运行时 `Fault`
  （`FAULT_BAD_OP`，同 `fire` 给坏外观的口径），**不建半成品**。
- `(x, y)` 是射线**原点**、`angle` 是方向（BAM）、`len` 是从原点到远端 `end` 的长度。
- `width` 是**判定宽度，也是画面宽度**（画多宽判多宽）；判定半高 = `width / 2`。
  负值/超界的几何量世界层会钳位并计一次违约，不必自己防。
- `warn / active / fade` 是三段时长（帧）：`warn` 帧预警（不判定）→ `active` 帧生效（判定）
  → `fade` 帧收缩（不判定）→ 回收。`warn = 0` 出生即生效；`fade = 0` 生效期一结束直接回收。
- 返回值是**打包激光句柄**：不透明值，别猜数值、别做算术；两个句柄相等 ⇔ 同一条激光。
  池满返回 `-1`（记 `pool_full[POOL_LASER]`，不 Fault）。

出生时的固定字段：`start = 0`、`end = start_len = len`、`speed = 0`、`omega = 0`——就是
"一条从原点长 `len` 的静止线"。要别的形态用下面的 `lz_*` 改。

## 形态一：预警线 → 扫射（`lz_omega`）

```ecl
// 竖一条 500px 的预警线，30 帧后生效 120 帧，再收缩 16 帧；
// 生效期以每秒 60 BAM（≈0.33°/帧）慢扫。
async sub sweep_laser() {
    var lz: int = laser(4, $self_x, $self_y, 90deg, 500.0fx, 32.0fx, 30, 120, 16);
    lz_omega(lz, 60bam);          // 每帧转多少 BAM；允许负（反向扫）
    wait(200);
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 300, 1, 1000, 1, sweep_laser);
    loop { wait(1); }
}
```

`lz_omega` 的 `a` 是**每帧**增量（`angle` 类型，BAM）。原作"每 N 帧转一次 `a`"的写法
对应 `lz_omega(lz, (a as int / N) as angle)`——见文末「和原作对照」。

## 形态二：自机狙（`lz_aim`）

`lz_aim(lz, off)` 把激光角度设为**从激光原点到自机 0 的方向 + `off`**。相对自机角的写法
是 `lz_aim(lz, a)`；要"发射瞬间就瞄"也可以直接给 `laser` 的角度位传
`aim_player() + a`（`aim_player()` 返 `angle`，加偏移角不判型报错）。

```ecl
// 预警 24 帧，线摆到自机方向 + 10°；生效 90 帧期间不再改角度，就是一条自机狙。
async sub aimed_laser() {
    var lz: int = laser(6, $self_x, $self_y, 0deg, 400.0fx, 24.0fx, 24, 90, 12);
    lz_aim(lz, 10deg);            // angle = atan2(自机 - 原点) + 10deg
    wait(150);
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 300, 1, 1000, 1, aimed_laser);
    loop { wait(1); }
}
```

## 形态三：飞出去的棒子（`lz_speed`）

`lz_speed(lz, speed, start_len)` 设速率与棒长：每帧 `end += speed`，同时
`start = max(start, end - start_len, 0)`——**近端跟着走**，所以看到的是一条定长棒子沿射线
飞出去，而不是无限长的线。`start` 越过 640 就出屏回收，不必自己 `lz_cancel`。

```ecl
// warn = 0：出生帧就生效；active 给足，等 start 越过 640 自动回收。
// 每帧长 4px、棒长 192px 的短棒沿 90° 方向飞出。
async sub spear() {
    var lz: int = laser(2, $self_x, $self_y, 90deg, 0.0fx, 6.0fx, 0, 9999, 0);
    lz_speed(lz, 4.0fx, 192.0fx);
    wait(240);
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 300, 1, 1000, 1, spear);
    loop { wait(1); }
}
```

顺带：`lz_start(lz, s)` 把近端留空 `s`（原作第 4 关的 `start = 64`），`start > end` 时把
`end` 抬到 `start`——想"从敌人身前一点开始射"就用它。

## 挂靠：跟着敌人走（`lz_anchor`）

`lz_anchor(lz, enemy, ox, oy)` 把激光挂到敌号 `enemy` 上：此后每帧原点 =
`敌位置 + (ox, oy)`，转场/走位时不用自己重发。传 `-1` 解除挂靠（原点留在原地）。
**代际不符（槽被新敌复用）或敌人已死会自动脱钩**，原点留在最后跟到的位置，不跳到新敌身上。

```ecl
// 挂在自己（生成这条激光的敌）身上，偏移 (0, 8)：像一个跟着走的小炮塔。
async sub turret() {
    var lz: int = laser(3, $self_x, $self_y, 90deg, 300.0fx, 16.0fx, 0, 180, 16);
    lz_anchor(lz, $self_enemy, 0.0fx, 8.0fx);   // $self_enemy 是 owner 敌的打包敌号
    wait(60);
    lz_rotate(lz, 45deg);                       // 挂在原地一次性转 45°
    wait(60);
    lz_cancel(lz);                              // 提前收掉
    wait(30);
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 300, 1, 1000, 1, turret);
    loop { wait(1); }
}
```

`lz_origin(lz, x, y)` 反过来直接设原点，并**解除挂靠**。`lz_alive(lz)` 探活（返 1/0，
只读不计数）——想复用句柄前先问一声，别对已经回收的槽瞎写。

## 坏句柄与降级

激光句柄带代际，槽被复用后旧句柄可辨。拿已回收/代际不符的句柄调任何 `lz_*`：

- `lz_alive` 返 `0`（只读，不计数）；
- 其余写口一律 **no-op + `contract_viol` +1**，不 Fault、不改到复用槽里的新激光。

`lz_anchor` 还有一个专门的降级：`enemy` 不是 `-1` 但已失效（死了 / 槽被复用 / 越界）时
不挂靠、计一次违约——**不会**顺手把你原来挂着的锚也解掉。

## 和原作对照（转写用）

- **宽度**：本引擎的 `width` 是画面宽度 = 判定宽度，判定半高 `width / 2`。原作 ZUN 的激光
  宽度参数 `w` 对应判定半高 `w / 4`（原作判定盒是 `[−w/4, +w/4]`）——要保持同样的判定半高，
  转写时把原作的值**减半**填进来。
- **`laser_index` / `laser_clear_all` 不需要**：那两条原作指令只是在维护敌人的激光指针表。
  这里有句柄——脚本把 `laser()` 的返回值存进局部变量就行；清激光用 `lz_cancel(lz)`（单条）
  或带 `FIELD_CLEAR_BULLETS` 的清弹区（全场，走 `clear_bullets()` / `clear_bullets_at()`）。
- **每 N 帧转一次 → `lz_omega`**：原作常见的
  `loop { laser_rotate(lz, a); wait(N); }` 就换成 `lz_omega(lz, (a as int / N) as angle)`
  （连续转，等价）。
- **每帧重发 `laser_offset` → `lz_anchor`**：原作靠每帧重设偏移跟随的写法，直接挂到敌号上。
- **85 / 86 的区别**（角度是否相对自机）不再是两条指令：写
  `laser(..., aim_player() + a, ...)` 或出生后 `lz_aim(lz, a)` 即可。
- **Extra 的"每帧重建"照原样转**（每帧 `laser()` + 短 `active`）：原作的判定就是那样，
  别合并成一条长激光。

---

**上一篇** ← [8 · 报错、静默降级与已知限制](8-errors.md) ·
**索引** → [`.ecl` 手册](../ecl-lang.md)
