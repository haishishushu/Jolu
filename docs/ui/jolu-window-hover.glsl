precision mediump float;
/** @resolution */
uniform vec2 u_resolution;
/** @mouse */
uniform vec2 u_mouse;
/** @label Content */
uniform sampler2D u_content;
/**
 * @label Hover background
 * @color
 * @default #F2F2F3
 */
uniform vec4 u_hoverColor;
/**
 * @label Close control
 * @default 0
 */
uniform float u_close;

void main() {
  vec4 content = texture2D(u_content, gl_FragCoord.xy / u_resolution);
  bool hovered = u_mouse.x > 0.0 && u_mouse.y > 0.0
    && u_mouse.x < u_resolution.x && u_mouse.y < u_resolution.y;
  if (hovered) {
    if (u_close > 0.5) content.rgb = vec3(content.a);
    vec4 background = vec4(u_hoverColor.rgb * u_hoverColor.a, u_hoverColor.a);
    content += background * (1.0 - content.a);
  }
  gl_FragColor = content;
}
