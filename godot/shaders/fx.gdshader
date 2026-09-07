shader_type canvas_item;
// fx 层:程序化一次性特效(表现契约 v2 §5.2)。一张 MultiMesh、stride 12,
// custom = [kind, age, param, sprite](effects.gd 每帧按 step 后帧号重算 age——不走壁钟)。
// 形状全是 t = age / life 的函数,CPU 侧零插值。QuadMesh 边长 = `quad_px`。
// kind:0 消弹淡出(采样弹图集格 sprite) / 1 爆炸环 / 2 命中火花 / 3 通用闪点 / 4 依附光环。
uniform sampler2D bullet_atlas : filter_nearest;
uniform float grid_cols = 16.0;
uniform float grid_rows = 12.0;
uniform float quad_px = 64.0;
// 各 kind 寿命(帧),与 content_tables.gd FX_LIFE 同值——改一边必须改另一边。
uniform vec4 life_0_3 = vec4(12.0, 20.0, 7.0, 30.0);
uniform float life_4 = 90.0;

varying flat vec4 cst;
varying flat float t;
varying flat float scale_px; // 本实例实际半边长(像素),给 fragment 换算 UV→像素

float life_of(float kind) {
	int k = int(kind + 0.5);
	if (k == 0) { return life_0_3.x; }
	if (k == 1) { return life_0_3.y; }
	if (k == 2) { return life_0_3.z; }
	if (k == 3) { return life_0_3.w; }
	if (k == 4) { return life_4; }
	return life_0_3.w;
}

void vertex() {
	cst = INSTANCE_CUSTOM;
	t = clamp(cst.y / max(life_of(cst.x), 1.0), 0.0, 1.0);
	int k = int(cst.x + 0.5);
	float s = 1.0;
	if (k == 0) {
		s = (16.0 / quad_px) * (1.0 + 0.6 * t); // 弹格 16px,淡出时微放大
	} else if (k == 2) {
		s = 16.0 / quad_px;                       // 火花:16px 内的小圆
	} else if (k == 4) {
		s = clamp(cst.z * 2.0, 8.0, 256.0) / quad_px; // 光环:param = 半径像素
	}
	VERTEX *= s;
	scale_px = s * quad_px * 0.5;
}

void fragment() {
	int k = int(cst.x + 0.5);
	vec2 p = (UV - 0.5) * 2.0 * scale_px; // 以像素为单位、中心为原点
	float r = length(p);
	vec4 c = vec4(0.0);
	if (k == 0) {
		// 消弹淡出:同一格弹图集原地淡出——看起来就是"那颗弹在消失"
		float s = mod(cst.w, grid_cols * grid_rows);
		vec2 cell = vec2(mod(s, grid_cols), floor(s / grid_cols));
		vec2 uv = (cell + UV) / vec2(grid_cols, grid_rows);
		c = texture(bullet_atlas, uv);
		c.a *= 1.0 - t;
		c.rgb += vec3(0.4) * (1.0 - t) * c.a; // 略提亮再散去
	} else if (k == 1) {
		float rr = 4.0 + 28.0 * t;
		float d = abs(r - rr);
		float a = (1.0 - smoothstep(0.8, 2.2, d)) * (1.0 - t);
		c = vec4(1.0, 0.8, 0.4, a);
	} else if (k == 2) {
		float rr = 2.0 + 6.0 * t;
		float a = (1.0 - smoothstep(rr - 1.0, rr, r)) * (1.0 - t);
		c = vec4(1.0, 1.0, 0.85, a);
	} else if (k == 4) {
		float rr = scale_px * 0.9;
		float d = abs(r - rr);
		float pulse = 0.6 + 0.4 * sin(cst.y * 0.4);
		float a = (1.0 - smoothstep(0.5, 2.5, d)) * (1.0 - t) * pulse;
		c = vec4(0.5, 0.9, 1.0, a);
	} else {
		// 通用闪点(kind 3 与一切未识别 kind):亮心圆盘,闪烁后散去
		float rr = 6.0 + 4.0 * sin(cst.y * 0.8);
		float a = (1.0 - smoothstep(rr - 1.5, rr, r)) * (1.0 - t);
		c = vec4(0.6, 1.0, 1.0, a);
	}
	COLOR = c; // 不乘 COLOR:MultiMesh 未开 use_colors 时它是垃圾(见 layer.gdshader 同处注释)
}
