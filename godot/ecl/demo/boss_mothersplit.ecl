// boss 第二张符卡:母弹分裂——boss 慢速横向游走(复用 boss_windchime.ecl 的 patrol(),
// boss_main 开局就 spawn 了它,全程贯穿两张卡,本文件不重复挂),周期性放一圈慢速母弹;
// 母弹飞行一段时间后自己消失,并在原地炸开成第二层子弹(子弹朝外扩散)。挂在 boss_main
// 的卡序第二位,一池总血 hp_threshold 递降表达(见 boss_windchime.ecl 顶注「两卡序」、
// spell_begin 调用处的注释)。
//
// **"母弹自己消失、原地炸开"没有专门的世界层机制**——P5(世界无回调)+ xform 的
// `SPAWN_PATTERN`(op 60)还是预留位(docs/xform-ops.md 表内标 📋),world 不会在弹死的
// 那一刻替你调回 ECL。这里用两条腿各管各的、靠同一个等待帧数 `MOTHER_LIFE` 对齐来凑出
// 同样的效果:
//   ① `xformdef` 挂 `set_life`,定时自爆(docs/xform-ops.md「定时自爆弹」范式)。
//   ② `sh_task` 给每颗母弹挂一个私有任务,等到同一帧数,在**当前任务的 owner(=这颗母弹
//      自己)**身上开一个新发射器——出弹点默认基点就是 owner 位置(`sh_offset` 全零时,
//      `self_pos` 按 owner_kind 派发:BULLET → 弹池坐标),不用手动读 `$self_x`/`$self_y`
//      再摆 `sh_offset_abs`,炸开的落点天然跟着母弹飞到哪炸到哪。
// 两条腿的帧数只要对得上,视觉上就是"同一时刻消失 + 炸开",不要求逐帧对齐到 tick 级
// (母弹自爆那一帧本来也在画面上看不出差一帧)。
const SPELL_MOTHERSPLIT: int = 2;

const MOTHER_LIFE: int = 110;         // 母弹存活帧数(≈1.83s)——①xformdef 自爆的对齐基准
const CHILD_WAYS: int = 12;           // 子弹固定路数(炸开密度不随难度变,只有母弹路数随难度变,见下)

// ⚠️ ②的 `sh_task` 任务不能也等 `MOTHER_LIFE` 帧——`sh_task`/`fire` 挂出来的任务"出生
// 当帧不跑"(docs/ecl-lang.md 五条坑之一),它的 `wait(MOTHER_LIFE)` 是从**出生后第 1
// 帧**才开始数的;而①的 xformdef 从**出生当帧**就开始跑(`add_speed` 那槽在创建帧的
// 变换相位就发了)。两条腿因此天生错开 1 帧——任务侧沿用同一个 `MOTHER_LIFE` 会在
// 母弹刚被 cleanup 回收**之后**才追到 `sh_fire`,那时 owner 已死,派发/调度门禁直接
// 拦下,子弹一颗都不会出现,而且不报错、不计数、什么都看不出来(拿 stg-harness run
// 逐帧 --at 扫描才抓到:母弹按时消失,但子弹从未出现过;完整记录见 docs/follow-ups.md
// 「F10」)。这里让任务提前 1 帧扣动扳机,用 `stg-harness run --at` 逐帧扫描过
// (差 1 帧刚好落空,差 2 帧仍稳定成功)。
const MOTHER_SPLIT_TASK_WAIT: int = MOTHER_LIFE - 1;

xformdef MOTHER_EXPIRE {
    // ⚠️ `@N` 挂在**它前面紧跟的那个 op 自己**头上,语义是"这个 op 发射后,再等 N 帧才
    // 执行下一槽"——不是"等 N 帧再执行这个 op"(docs/xform-ops.md「wait:发射本 op 后
    // 恰等 wait 帧再执行下一槽」;「定时自爆弹」范式例子的 `SET_SPEED` 那槽正是把 wait
    // 挂在**前一槽**、留给自己 wait=0 立即发)。单槽 `xformdef { @110 set_life(1); }`
    // 会在创建当帧就把 set_life 发出去——母弹活不过一帧就自爆,这是踩过的坑,详见
    // docs/follow-ups.md「F10」(2026-08-02 母弹分裂卡验证时用 stg-harness run 抓到)。
    // 正确写法是拿一个**无副作用的占位 op** 扛这个 wait 字段,推迟到下一槽:
    // `add_speed(0fx)` 净效果为零(速度不变),纯粹借它的 wait 撑出 MOTHER_LIFE 帧延迟。
    @110 add_speed(0fx);
    // 延迟结束后立即发:下一帧寿命减到 0,母弹连同这份 xform 段一起被 cleanup 回收。
    // 母弹本身不转向不变速,分裂前的轨迹是直线径向飞出。
    set_life(1);
}

// 挂在每颗母弹自己身上的私有任务(sh_task 派发,owner=该弹,每颗母弹一份)。等到与
// xformdef 同一帧数后,在母弹当前位置开一圈子弹,朝各自角度扩散出去。
async sub mother_split_task() {
    wait(MOTHER_SPLIT_TASK_WAIT);
    // 槽 2:这是本任务(挂在母弹上)自己私有的 4 个 shooter 槽之一,与 motherspell_pattern
    // 那个任务的槽 0 各自独立、不冲突(发射器槽是每任务私有的,见 boss_windchime.ecl 顶注)。
    sh_reset(2);
    sh_sprite(2, BALL, COLOR_ORANGE);
    sh_ring(2, 1);              // 整周环,引擎均分,不手算步长(掉余数的坑见 4-bullets.md)
    sh_count(2, CHILD_WAYS, 1);
    sh_speed(2, 1.7fx, 0fx);    // 子弹比母弹快,炸开的观感要和母弹的慢速漂移明显区分开
    sh_fire(2);                 // 出弹点默认 = owner(母弹自己)当前位置,原地炸开
}

async sub motherspell_pattern() {
    sh_reset(0);
    sh_sprite(0, AMULET, COLOR_DARK_BLUE); // 大型弹型,一眼认出这是"母弹",与子弹的 BALL 区分
    sh_ring(0, 1);
    sh_speed(0, 0.8fx, 0fx);    // 母弹慢速扩散——玩家要有时间数清路数、找好安全缝再等它炸
    sh_xform(0, MOTHER_EXPIRE);
    sh_task(0, mother_split_task);

    var base: angle = 0deg;
    loop {
        // 难度档越高母弹路数越多:Easy 10 路,每档 +2,Lunatic 16 路——子弹路数(CHILD_WAYS)
        // 钉死不随难度变,难度只体现在"要躲的母弹环更密",不体现在"炸开更密"。
        var ways: int = 10 + global(GVAR_RANK) * 2;
        sh_count(0, ways, 1);
        sh_angle(0, base, 0deg);
        sh_fire(0);
        base = base + 11deg; // 逐轮转 11°,环不会帧帧重叠在同一批缝隙上(同风铃卡的错位手法)
        wait(85);
    }
}
