// demo 局主编排。多文件单元:目录整取按名排序(boss_windchime < main < stage1),入口 main。
// 转场协议(壳子刀 2026-09-07):风铃卡后 stage_clear(1) 发 EVT_STAGE_CLEARED 事实事件并让出一帧,
// 宿主停拍(世界时间线冻结);下一条语句在玩家确认后的第一帧才跑——这里没有下一关,驻留。
sub main() {
    bgm(1);
    bg(1);
    mark(1); // 杂兵段练习位
    stage1();
    mark(2) { // boss 段练习位。跳入补偿(自动):bg(1);块内手写的 bgm/bg_phase 抑制同类注入
        bgm(2);
        bg_phase(1); // boss 战背景停滚
    }
    boss_battle();
    bg_phase(0);
    add_score(100000); // 关底 bonus 世界内入账(结算数字在停拍前定格)
    stage_clear(1); // = SYS 723 + wait(1):事件走通道 A,不再是 emit_req 挂牌
    loop { wait(600); } // 停拍后驻留(没有下一关)
}
