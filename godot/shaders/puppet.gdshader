shader_type canvas_item;
// 敌人木偶(Sprite2D)的受击闪白:`flash` 由 puppets().hit_flash 列每帧设(表现契约 v2 §5.1)。
// 每个木偶一份 ShaderMaterial 副本(256 份可接受),uniform 逐节点独立。
uniform float flash : hint_range(0.0, 1.0) = 0.0;

void fragment() {
	vec4 tex = texture(TEXTURE, UV);
	COLOR = vec4(mix(tex.rgb, vec3(1.0), flash), tex.a) * COLOR;
}
