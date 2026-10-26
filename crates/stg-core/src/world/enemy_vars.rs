//! 敌人脚本共享数值槽；只接受世界层句柄，与任务生命周期无关。
use super::{STATUS_BAD_ARGS, STATUS_OK, STATUS_STALE_HANDLE, WorldBody};
use crate::enemy::{ENEMY_FX_SLOTS, ENEMY_INT_SLOTS, EnemyHandle};
use crate::math::Fx;

impl WorldBody {
    fn enemy_script_slot(
        &mut self,
        h: EnemyHandle,
        slot: i32,
        count: usize,
    ) -> Option<(usize, usize)> {
        let Some(i) = self.enemies.get(h) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            return None;
        };
        let Some(slot) = usize::try_from(slot).ok().filter(|&s| s < count) else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return None;
        };
        self.last_status = STATUS_OK;
        Some((i, slot))
    }
    pub(crate) fn enemy_script_int(&mut self, h: EnemyHandle, slot: i32) -> i32 {
        let Some((i, slot)) = self.enemy_script_slot(h, slot, ENEMY_INT_SLOTS) else {
            return 0;
        };
        self.enemies.script_int[i][slot]
    }
    pub(crate) fn set_enemy_script_int(&mut self, h: EnemyHandle, slot: i32, value: i32) {
        if let Some((i, slot)) = self.enemy_script_slot(h, slot, ENEMY_INT_SLOTS) {
            self.enemies.script_int[i][slot] = value;
        }
    }
    pub(crate) fn enemy_script_fx(&mut self, h: EnemyHandle, slot: i32) -> Fx {
        let Some((i, slot)) = self.enemy_script_slot(h, slot, ENEMY_FX_SLOTS) else {
            return Fx::ZERO;
        };
        self.enemies.script_fx[i][slot]
    }
    pub(crate) fn set_enemy_script_fx(&mut self, h: EnemyHandle, slot: i32, value: Fx) {
        if let Some((i, slot)) = self.enemy_script_slot(h, slot, ENEMY_FX_SLOTS) {
            self.enemies.script_fx[i][slot] = value;
        }
    }
}
