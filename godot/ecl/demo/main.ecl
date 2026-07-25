// demo 局主编排。多文件单元:目录整取按名排序(boss_windchime < main < stage1),入口 main。
// 转场协议:风铃卡后 emit REQ_STAGE_CLEAR 挂牌,宿主停拍(世界时间线冻结),脚本驻留。
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
    add_score(100000); // 关底 bonus 世界内入账(结算数字在挂牌前定格)
    emit_req(REQ_STAGE_CLEAR, 1, 0, 0, 0, 0, 0);
    loop { wait(600); } // 挂牌后驻留
}
