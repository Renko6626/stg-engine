shader_type canvas_item;
// 影子层(観測,时间机制内核刀 spec §5):影子世界的弹,布局同 layer.gdshader(custom.x = 格号)。
// 策划案 7.4「预测/过去永远比实体暗、比实体细」:去饱和 + 压暗 + 半透明,不闪光不放大。
// 真线框等美术;这里先满足"一眼分得出这不是现在"。
uniform sampler2D atlas : filter_nearest;
uniform float grid_cols = 16.0;
uniform float grid_rows = 12.0;
uniform float dim = 0.55;
uniform float alpha = 0.5;

varying flat vec2 cell;

void vertex() {
	float s = INSTANCE_CUSTOM.x;
	float total = grid_cols * grid_rows;
	s = mod(s, total);
	cell = vec2(mod(s, grid_cols), floor(s / grid_cols));
}

void fragment() {
	vec2 cell_uv = (cell + UV) / vec2(grid_cols, grid_rows);
	vec4 tex = texture(atlas, cell_uv);
	float lum = dot(tex.rgb, vec3(0.3, 0.59, 0.11));
	vec3 rgb = mix(tex.rgb, vec3(lum), 0.6) * dim;
	COLOR = vec4(rgb, tex.a * alpha); // 不乘 COLOR(同 layer.gdshader 判决)
}
