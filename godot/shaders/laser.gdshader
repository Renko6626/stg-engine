shader_type canvas_item;
// 激光层(激光池刀 2026-09-25 Task 6;spec §7;render-contract「激光层」一节)。
//
// **不采样图集**:截面渐变程序化生成。实例布局 stride 12 同其它层,但**基自带缩放**
// (playfield.gd 的 QuadMesh 取 1×1):
//   局部 x 轴 = 横截面方向 × 显示宽度;局部 y 轴 = 激光方向 × (end − start);
//   原点 = 线段中点;custom = [color 0..15, alpha, 0, 0]。
// 截面:中心白芯(提亮到白),向两侧按颜色表渐暗;加色混合,alpha 乘 custom.y。
// 预警细线/最后 min(warn,30) 帧长宽/收缩变窄或淡出全在 CPU 侧算(crates/stg-godot/src/frame.rs
// 的 `laser_display`),shader 只负责把实例颜色画成一条有截面的光带。
//
// 16 色表照 `godot/ecl/game/bullets.ecl` 的**颜色列顺序**(sprite = 弹型 × 16 + 色号)。
// RGB 是从 `godot/assets/bullets.png` 各色列采样的高饱和代表色,是**近似占位**——原作是
// etama3 的 8 色截面小方块;图集补一行真正的截面渐变贴图后,本表应改为采样那张图。
// (待办见 docs/follow-ups.md「激光」条目。)
render_mode blend_add;

const vec3 LASER_COLORS[16] = {
	vec3(0.643, 0.643, 0.643), // 0  gray
	vec3(0.667, 0.157, 0.157), // 1  dark red
	vec3(1.000, 0.000, 0.000), // 2  red
	vec3(0.667, 0.157, 0.667), // 3  dark pink
	vec3(1.000, 0.314, 1.000), // 4  pink
	vec3(0.000, 0.000, 0.784), // 5  dark blue
	vec3(0.000, 0.157, 1.000), // 6  blue
	vec3(0.000, 0.784, 0.973), // 7  dark cyan
	vec3(0.000, 0.973, 0.973), // 8  cyan
	vec3(0.157, 0.706, 0.353), // 9  dark green
	vec3(0.000, 0.973, 0.471), // 10 green
	vec3(0.471, 0.973, 0.188), // 11 lime
	vec3(0.784, 0.863, 0.235), // 12 yellow-green
	vec3(1.000, 0.941, 0.000), // 13 yellow
	vec3(1.000, 0.627, 0.157), // 14 orange
	vec3(1.000, 1.000, 1.000) // 15 white
};

varying flat float color_idx;
varying flat float alpha;

void vertex() {
	color_idx = INSTANCE_CUSTOM.x;
	alpha = INSTANCE_CUSTOM.y;
}

void fragment() {
	int idx = clamp(int(color_idx + 0.5), 0, 15);
	vec3 base = LASER_COLORS[idx];
	float d = abs(UV.x - 0.5) * 2.0; // 0 = 截面中心,1 = 边缘
	float core = 1.0 - d;
	float a = core * alpha;
	vec3 rgb = mix(base, vec3(1.0), core * core); // 中心白芯
	// 加色混合:rgb 预乘 alpha、输出 alpha 恒 1——这样无论 Godot 的 blend_add 用
	// (SRC_ALPHA,ONE) 还是 (ONE,ONE) 都得到 "rgb·a + dst"(见 PR 说明)。
	COLOR = vec4(rgb * a, 1.0);
}
