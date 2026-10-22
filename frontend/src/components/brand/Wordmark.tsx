/**
 * Jolu 艺术字标（Jolu.pen「Jolu 艺术字标 · 圆润连环」）：
 * 微倾斜圆润笔画，绿色闭环 o 呼应 Join + Loop。路径与线宽原样取自设计稿。
 */
export function Wordmark({ width = 54, height = 22 }: { width?: number; height?: number }) {
  return (
    <svg
      role="img"
      aria-label="Jolu"
      width={width}
      height={height}
      viewBox="0 0 67 27"
      preserveAspectRatio="none"
      fill="none"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0"
    >
      <g stroke="var(--jolu-text)" strokeWidth={2.6} vectorEffect="non-scaling-stroke">
        <path d="M4 4h11v12c0 9-12 9-12 1" vectorEffect="non-scaling-stroke" />
        <path d="M44 3l-4 15q-1 6 5 4" vectorEffect="non-scaling-stroke" />
        <path d="M51 11l-2 7q-1 6 5 5 5-1 7-12m0 0l-2 10q0 3 4 1" vectorEffect="non-scaling-stroke" />
      </g>
      <path d="M29 10c-9 0-11 13-3 13 9 0 12-13 3-13z" stroke="var(--jolu-accent)" strokeWidth={2.6} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}
