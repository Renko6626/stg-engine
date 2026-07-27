// 杂兵段:三波×四机俯冲,瞄准三连发,退场后越出回收线由 OOB 判定杀死(cleanup.rs,
// 回收线 = FIELD_HEIGHT(448)+ENEMY_OOB_MARGIN(256)=704,退场目标 y 须探到线外——
// 原 560.0fx 够不着线,敌会一直停在场内不回收)。方向注意是"敌死→任务随之终止"
// 单向(spawn_enemy 文档:敌死任务亡),不是"zako_dive 跑完 return 反过来杀敌";
// 实机路径是敌在 150 帧移动途中越线被回收,末尾这句 `wait(600)` 敌先死、任务先随之
// 终止,实际跑不到。
async sub zako_dive() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(OUTLINE, COLOR_CYAN_DARK, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 760.0fx, 1);
    wait(600);
}

sub stage1() {
    for w in 0..3 {
        for i in 0..4 {
            _ = spawn_enemy((i * 96 - 144) as fx, 2.0fx, 40, 1, 300, 0, zako_dive);
            wait(15);
        }
        wait(150);
    }
    wait(120); // 清场缓冲
}
