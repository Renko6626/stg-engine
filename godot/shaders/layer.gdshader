shader_type canvas_item;
// 图集选格:INSTANCE_CUSTOM.x = sprite 号 = 格号(行优先;render-contract §3)。
// custom.y 在弹层 = 弹龄(step 后帧号 − born_frame,首帧 1;表现契约 v2 §2),其余层恒 0;
// custom.z/w 保留空位(scale/alpha/调色),stride 12 冻结。
uniform sampler2D atlas : filter_nearest;
uniform float grid_cols = 8.0;
uniform float grid_rows = 1.0;
// 出现闪光(表现契约 v2 §7 "弹出现闪光"):age < spawn_flash_frames 时放大并提亮,数帧内收敛。
// **0 = 关闭**——shots/items 层 custom.y 恒 0,不设本 uniform 就永远不会闪;只有弹层设 6。
uniform float spawn_flash_frames = 0.0;

varying flat vec2 cell;
varying flat float flash; // 0..1,1 = 刚出生

void vertex() {
	float s = INSTANCE_CUSTOM.x;
	float total = grid_cols * grid_rows;
	s = mod(s, total); // 越界号回卷(表现层容错,render-contract §3)
	cell = vec2(mod(s, grid_cols), floor(s / grid_cols));
	flash = 0.0;
	if (spawn_flash_frames > 0.0) {
		float age = INSTANCE_CUSTOM.y;
		flash = 1.0 - clamp((age - 1.0) / spawn_flash_frames, 0.0, 1.0);
		VERTEX *= 1.0 + 0.8 * flash;
	}
}

void fragment() {
	vec2 cell_uv = (cell + UV) / vec2(grid_cols, grid_rows);
	vec4 tex = texture(atlas, cell_uv);
	tex.rgb = mix(tex.rgb, vec3(1.0), 0.6 * flash * tex.a);
	// **不乘 COLOR**(表现契约 v2 有头目验判决,2026-09-07):MultiMesh 未开 use_colors 时片元
	// COLOR 输入是未定义的逐像素垃圾(GL/Vulkan 两后端一致的彩色噪点),乘上去弹就成了碎彩点。
	// 这是场景刀以来一直存在的 bug——16px 的弹看着像"彩色小球",B23 那次目验没辨出来。
	// 要整层调色/淡出,走 uniform 或 custom.z/w,别碰 COLOR。
	COLOR = tex;
}
