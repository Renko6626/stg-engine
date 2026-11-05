# Xform 单值 STEP 扩展

用户已确认：新增 `step_vx`、`step_vy`、`step_ax`、`step_ay`、`step_accel`、`step_ang_vel`；冲突保持固定槽顺序写入，不提供检测、取消、接管或恢复机制。混用冲突属性不受支持。

- 每条操作为 `(target, frames + 65536 * easing)`，占两槽；启动当帧记录起点，次帧开始推进，终帧写精确目标；零时长立即设置。沿用等待、段尾、动态参数和容量规则。
- `step_ang_vel` 目标为 angle，按有符号 i16 角速度插值，不走最短弧；其余新增目标为 fx，时长和曲线打包参数为 int。
- 编号：44 STEP_ANG_VEL、45 STEP_ACCEL；70 STEP_VX、71 STEP_VY、72 STEP_AX、73 STEP_AY。原编号不变，ENGINE_VER 28→29，World 布局不变。
- 复用现有 STEP scratch：起点与 active/elapsed。共享已有插值公式，STEP_ANGLE 保留最短弧。
- VX/VY 写入只改对应速度分量，然后回填 speed/angle；不切换运动模式。AX/AY 调用已有 gravity setter，保留另一加速度分量；ACCEL/ANG_VEL 调用各自已有 setter，沿用既有模式切换语义。
- 各活跃 STEP 按物理槽升序执行，随后执行当帧序列操作，再进入既有运动积分。没有隐式取消；stop_fx 不取消 STEP，参数 STEP 后续写入可以重新开启其运动模式。作者负责避免属性竞争。
- 不增加依赖、弹池字段、通用异步框架、组合 STEP_GRAVITY、追踪或相对多圈转向。

验证覆盖新增属性的起点/中间值/终点、单轴保持另一轴、负角速度、零时长、非线性曲线及真实 ECL 静态/动态接入；运行已有相关测试、fmt/clippy、表校验与本机 golden。不为不受支持的冲突输入增加测试或防御机制。
