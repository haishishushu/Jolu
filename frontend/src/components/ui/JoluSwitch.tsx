export interface JoluSwitchProps {
  on: boolean
  onChange?: (next: boolean) => void
  disabled?: boolean
  label: string
  compact?: boolean
}

/** 轨道与命中区分离：视觉保持紧凑，点击区域不侵入相邻控件。 */
export function JoluSwitch({ on, onChange, disabled, label, compact = false }: JoluSwitchProps) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label}
      disabled={disabled || !onChange} onClick={() => onChange?.(!on)}
      data-size={compact ? "compact" : "regular"} className="jolu-switch">
      <span aria-hidden className="jolu-switch-track">
        <span className="jolu-switch-thumb" />
      </span>
    </button>
  )
}
