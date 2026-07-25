shader_type canvas_item;
// 图集选格:INSTANCE_CUSTOM.x = sprite 号 = 格号(行优先;render-contract §3)。
// custom.y/z/w 保留空位(scale/alpha/调色),stride 12 冻结。
uniform float grid_cols = 8.0;
uniform float grid_rows = 1.0;

varying flat vec2 cell;

void vertex() {
	float s = INSTANCE_CUSTOM.x;
	float total = grid_cols * grid_rows;
	s = mod(s, total); // 越界号回卷(表现层容错,render-contract §3)
	cell = vec2(mod(s, grid_cols), floor(s / grid_cols));
}

void fragment() {
	vec2 uv = (cell + UV) / vec2(grid_cols, grid_rows);
	COLOR = texture(TEXTURE, uv);
}
