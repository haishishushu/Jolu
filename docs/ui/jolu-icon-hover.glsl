precision mediump float;

/** @resolution */
uniform vec2 u_resolution;
/** @mouse */
uniform vec2 u_mouse;
/** @label Content */
uniform sampler2D u_content;
/**
 * @label Icon center X
 * @default 18
 */
uniform float u_iconX;
/**
 * @label Hover color
 * @color
 * @default #E7F7EE
 */
uniform vec4 u_hoverColor;

void main() {
  vec2 p = gl_FragCoord.xy;
  bool hovered = u_mouse.x > 0.0 && u_mouse.y > 0.0
    && u_mouse.x < u_resolution.x && u_mouse.y < u_resolution.y;
  vec4 content = texture2D(u_content, p / u_resolution);
  if (hovered) {
    vec2 q = abs(p - u_resolution * 0.5) - (u_resolution * 0.5 - 8.0);
    float distanceToEdge = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - 8.0;
    float mask = 1.0 - smoothstep(-0.75, 0.75, distanceToEdge);
    vec4 backdrop = vec4(u_hoverColor.rgb * u_hoverColor.a, u_hoverColor.a) * mask;
    content += backdrop * (1.0 - content.a);
  }
  gl_FragColor = content;
}
