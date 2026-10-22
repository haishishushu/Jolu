use tauri::image::Image;

/// 系统托盘图标的基准边长：Windows 在 100% DPI 下索取 16px，
/// 之后随缩放线性放大（125%→20、150%→24、175%→28、200%→32，以此类推）。
const BASE_TRAY_SIZE: usize = 16;

/// 预烘焙的托盘底图档位（未编码 RGBA，由 scripts/icons/generate.mjs 生成）。
///
/// 壳层按 WM_DISPLAYNAME 的 SM_CXSMICON 尺寸绘制托盘图标：喂 32px 而系统要 16px
/// 时会被二次缩小，这正是托盘发糊的来源。按系统实际尺寸出图后全程零缩放；
/// 档位覆盖 100%–250% 缩放，极端 DPI 下退化到最近一档，只余一次轻微采样。
static BASE_16: &[u8] = include_bytes!("../icons/tray-base-16.rgba");
static BASE_20: &[u8] = include_bytes!("../icons/tray-base-20.rgba");
static BASE_24: &[u8] = include_bytes!("../icons/tray-base-24.rgba");
static BASE_28: &[u8] = include_bytes!("../icons/tray-base-28.rgba");
static BASE_32: &[u8] = include_bytes!("../icons/tray-base-32.rgba");
static BASE_36: &[u8] = include_bytes!("../icons/tray-base-36.rgba");
static BASE_40: &[u8] = include_bytes!("../icons/tray-base-40.rgba");
const _: () = assert!(BASE_16.len() == 16 * 16 * 4, "tray-base-16.rgba 档位不符");
const _: () = assert!(BASE_20.len() == 20 * 20 * 4, "tray-base-20.rgba 档位不符");
const _: () = assert!(BASE_24.len() == 24 * 24 * 4, "tray-base-24.rgba 档位不符");
const _: () = assert!(BASE_28.len() == 28 * 28 * 4, "tray-base-28.rgba 档位不符");
const _: () = assert!(BASE_32.len() == 32 * 32 * 4, "tray-base-32.rgba 档位不符");
const _: () = assert!(BASE_36.len() == 36 * 36 * 4, "tray-base-36.rgba 档位不符");
const _: () = assert!(BASE_40.len() == 40 * 40 * 4, "tray-base-40.rgba 档位不符");

const TRAY_BASES: [(usize, &[u8]); 7] = [
    (16, BASE_16),
    (20, BASE_20),
    (24, BASE_24),
    (28, BASE_28),
    (32, BASE_32),
    (36, BASE_36),
    (40, BASE_40),
];

/// 按窗口缩放倍数算出系统托盘索取的边长（SM_CXSMICON = 16 × DPI / 96），
/// 再吸附到最近的烘焙档。超出烘焙范围的 DPI（>250%）封顶到最大档。
pub fn output_size(scale_factor: f64) -> usize {
    let wanted = (BASE_TRAY_SIZE as f64 * scale_factor).round().clamp(16.0, 40.0) as usize;
    TRAY_BASES
        .iter()
        .min_by_key(|(size, _)| size.abs_diff(wanted))
        .map(|(size, _)| *size)
        .unwrap_or(40)
}

/// 托盘底图。请求尺寸与烘焙档完全一致时全程零缩放；
/// 仅极端 DPI 才取最近一档，由壳层做唯一一次采样。
pub fn base(size: usize) -> Image<'static> {
    let (baked, rgba) = TRAY_BASES
        .iter()
        .min_by_key(|(baked, _)| baked.abs_diff(size))
        .unwrap_or(&TRAY_BASES[TRAY_BASES.len() - 1]);
    Image::new(rgba, *baked as u32, *baked as u32)
}

/// 角标几何，以 32px 画布为基准描述，再按实际输出尺寸等比缩放（Jolu.pen 画板 108）。
const BADGE_CENTER: f64 = 23.7;
const BADGE_RADIUS: f64 = 5.1;
const BADGE_RING: f64 = 6.65;

/// 托盘图标：人物底图 + 右下角额度状态圆点（绿 正常 / 黄 接近上限 / 红 已耗尽 / 灰 未知），
/// 白色描边在深浅任务栏都能辨认。底图与输出同尺寸，人物像素原样保留不再采样；
/// 只有圆点与描边按 4×4 超采样抗锯齿，边缘清楚不发糊。
pub fn badge(base: &Image<'_>, color: [u8; 3]) -> Image<'static> {
    let size = base.width() as usize;
    let scale = size as f64 / 32.0;
    let center = BADGE_CENTER * scale;
    let radius = BADGE_RADIUS * scale;
    let ring = BADGE_RING * scale;
    let source = base.rgba();

    let mut pixels = source.to_vec();
    for y in 0..size {
        for x in 0..size {
            // 只处理圆点外接框内的像素，其余保持底图原样
            if (x as f64 + 0.5 - center).abs() > ring + 1.0 || (y as f64 + 0.5 - center).abs() > ring + 1.0 {
                continue;
            }
            let i = (y * size + x) * 4;
            let under = [source[i] as f64, source[i + 1] as f64, source[i + 2] as f64, source[i + 3] as f64 / 255.0];
            let mut sum = [0.0f64; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f64 + (sx as f64 + 0.5) / 4.0;
                    let py = y as f64 + (sy as f64 + 0.5) / 4.0;
                    let distance = (px - center).hypot(py - center);
                    // 预乘累加：[r*a, g*a, b*a, a]，圆点与描边不透明（a = 1）
                    let sample = if distance <= radius {
                        [color[0] as f64, color[1] as f64, color[2] as f64, 1.0]
                    } else if distance <= ring {
                        [255.0, 255.0, 255.0, 1.0]
                    } else {
                        [under[0] * under[3], under[1] * under[3], under[2] * under[3], under[3]]
                    };
                    for channel in 0..4 {
                        sum[channel] += sample[channel];
                    }
                }
            }
            if sum[3] > 0.0 {
                for channel in 0..3 {
                    pixels[i + channel] = (sum[channel] / sum[3]).round().clamp(0.0, 255.0) as u8;
                }
                pixels[i + 3] = (sum[3] / 16.0 * 255.0).round() as u8;
            } else {
                pixels[i..i + 4].copy_from_slice(&[0; 4]);
            }
        }
    }
    Image::new_owned(pixels, size as u32, size as u32)
}

/// 任务栏图标的基准边长：Windows 11 任务栏在 100% 缩放下按 24px 绘制，随缩放线性放大。
const BASE_TASKBAR_SIZE: f64 = 24.0;

static TASKBAR_24: &[u8] = include_bytes!("../icons/taskbar-24.rgba");
static TASKBAR_30: &[u8] = include_bytes!("../icons/taskbar-30.rgba");
static TASKBAR_36: &[u8] = include_bytes!("../icons/taskbar-36.rgba");
static TASKBAR_42: &[u8] = include_bytes!("../icons/taskbar-42.rgba");
static TASKBAR_48: &[u8] = include_bytes!("../icons/taskbar-48.rgba");
const _: () = assert!(TASKBAR_24.len() == 24 * 24 * 4, "taskbar-24.rgba 档位不符");
const _: () = assert!(TASKBAR_30.len() == 30 * 30 * 4, "taskbar-30.rgba 档位不符");
const _: () = assert!(TASKBAR_36.len() == 36 * 36 * 4, "taskbar-36.rgba 档位不符");
const _: () = assert!(TASKBAR_42.len() == 42 * 42 * 4, "taskbar-42.rgba 档位不符");
const _: () = assert!(TASKBAR_48.len() == 48 * 48 * 4, "taskbar-48.rgba 档位不符");

const TASKBAR_BASES: [(usize, &[u8]); 5] = [
    (24, TASKBAR_24),
    (30, TASKBAR_30),
    (36, TASKBAR_36),
    (42, TASKBAR_42),
    (48, TASKBAR_48),
];

/// 主窗口的任务栏图标：按窗口缩放挑选原尺寸底图，系统直接绘制、不再缩放。
/// 锁定版本的 Tauri 默认取 ICO 第一张作窗口图标，任务栏会把它缩放成发虚的一团。
pub fn taskbar(scale_factor: f64) -> Image<'static> {
    let wanted = (BASE_TASKBAR_SIZE * scale_factor).round() as usize;
    let (size, rgba) = TASKBAR_BASES
        .iter()
        .min_by_key(|(size, _)| size.abs_diff(wanted))
        .unwrap_or(&TASKBAR_BASES[TASKBAR_BASES.len() - 1]);
    Image::new(rgba, *size as u32, *size as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(icon: &Image<'_>, x: usize, y: usize) -> [u8; 4] {
        let w = icon.width() as usize;
        let i = (y * w + x) * 4;
        icon.rgba()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn output_size_follows_system_small_icon_metric() {
        assert_eq!(output_size(1.0), 16);
        assert_eq!(output_size(1.25), 20);
        assert_eq!(output_size(1.5), 24);
        assert_eq!(output_size(1.75), 28);
        assert_eq!(output_size(2.0), 32);
        assert_eq!(output_size(2.5), 40);
        // 超出烘焙档的极端 DPI 封顶，非标准缩放吸附到最近一档。
        assert_eq!(output_size(3.0), 40);
        assert_eq!(output_size(1.1), 16);
    }

    #[test]
    fn base_matches_the_requested_tier_exactly() {
        for size in [16, 20, 24, 28, 32, 36, 40] {
            let base = base(size);
            assert_eq!(base.width(), size as u32);
            assert_eq!(base.height(), size as u32);
            assert_eq!(base.rgba().len(), size * size * 4);
        }
    }

    #[test]
    fn embedded_base_contains_the_jolu_character_palette() {
        // 防止生成脚本误用旧灵动岛底图：人物应同时包含肤色、绿色配饰和透明背景。
        for size in [16, 32, 40] {
            let base = base(size);
            let mid = size / 2;
            let i = (mid * size + mid) * 4;
            let pixel = &base.rgba()[i..i + 4];
            // 人物 PNG 的半透明抗锯齿会进入小尺寸采样，中心应接近不透明。
            assert!(pixel[3] >= 240, "{size}px 人物中心应接近不透明，实际 {pixel:?}");
            assert!(base.rgba().chunks_exact(4).any(|p| p[3] > 200 && p[0] > 150 && p[0] as u16 > p[1] as u16 + 20 && p[0] as u16 > p[2] as u16 + 20), "{size}px 缺少人物肤色");
            assert!(base.rgba().chunks_exact(4).any(|p| p[3] > 200 && p[1] as u16 > p[0] as u16 + 20 && p[1] as u16 > p[2] as u16 + 10), "{size}px 缺少绿色配饰");
            assert_eq!(at(&base, 0, 0)[3], 0, "人物底图角点应透明");
        }
    }

    #[test]
    fn badge_keeps_the_character_and_draws_a_ringed_status_dot() {
        let base = base(32);
        let icon = badge(&base, [6, 193, 103]);
        assert_eq!(icon.width(), 32);
        assert_eq!(at(&icon, 23, 23), [6, 193, 103, 255], "圆点中心应为状态色");
        assert_eq!(at(&icon, 29, 23), [255, 255, 255, 255], "圆点外圈应为白色描边");
        assert_eq!(at(&icon, 16, 16), at(&base, 16, 16), "圆点以外的人物像素应原样保留");
        assert_eq!(at(&icon, 0, 0)[3], 0, "角点应透明");
    }

    #[test]
    fn badge_stays_visible_at_the_native_tray_sizes() {
        for size in [16, 20, 24] {
            let icon = badge(&base(size), [220, 38, 38]);
            assert_eq!(icon.width(), size as u32);
            let center = (23.7 * size as f64 / 32.0) as usize;
            assert_eq!(at(&icon, center, center)[..3], [220, 38, 38], "{size}px 圆点中心应为状态色");
            let ring = (center + 1..size).map(|x| at(&icon, x, center)).any(|p| p[0] > 220 && p[1] > 220 && p[2] > 220);
            assert!(ring, "{size}px 下白色描边应可见");
        }
    }

    #[test]
    fn taskbar_icon_matches_the_system_scale() {
        for (scale, size) in [(1.0, 24), (1.25, 30), (1.5, 36), (1.75, 42), (2.0, 48), (3.0, 48)] {
            let icon = taskbar(scale);
            assert_eq!(icon.width(), size, "{scale} 倍缩放");
            assert_eq!(icon.rgba().len(), (size * size * 4) as usize);
            assert_eq!(at(&icon, 0, 0)[3], 0, "任务栏底图角点应透明");
        }
    }
}
