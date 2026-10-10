use image::{DynamicImage, imageops::FilterType};
use velo_domain::Palette;

type Lab = [f32; 3];
fn linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn gamma(v: f32) -> f32 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn lab(rgb: [u8; 3]) -> Lab {
    let [r, g, b] = rgb.map(|v| linear(v as f32 / 255.0));
    let l = (0.41222146 * r + 0.53633255 * g + 0.051445995 * b).cbrt();
    let m = (0.2119035 * r + 0.6806995 * g + 0.10739696 * b).cbrt();
    let s = (0.08830246 * r + 0.28171885 * g + 0.6299787 * b).cbrt();
    [
        0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    ]
}
fn rgb([l, a, b]: Lab) -> [u8; 3] {
    let x = (l + 0.39633778 * a + 0.21580376 * b).powi(3);
    let y = (l - 0.105561346 * a - 0.06385417 * b).powi(3);
    let z = (l - 0.08948418 * a - 1.2914855 * b).powi(3);
    [
        4.0767417 * x - 3.3077116 * y + 0.23096994 * z,
        -1.268438 * x + 2.6097574 * y - 0.34131938 * z,
        -0.0041960863 * x - 0.7034186 * y + 1.7076147 * z,
    ]
    .map(|v| (gamma(v.clamp(0.0, 1.0)).clamp(0.0, 1.0) * 255.0).round() as u8)
}
fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}
fn chroma(lab: Lab) -> f32 {
    lab[1].hypot(lab[2])
}
fn distance(a: Lab, b: Lab) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}
fn luminance(rgb: [u8; 3]) -> f32 {
    let [r, g, b] = rgb.map(|v| linear(v as f32 / 255.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}
pub(crate) fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn composite(foreground: [u8; 3], background: [u8; 3], alpha: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (foreground[i] as f32 * alpha + background[i] as f32 * (1.0 - alpha)).round() as u8
    })
}

fn ink(color: Lab, veil: [u8; 3], bounds: [[u8; 3]; 2], light: bool) -> ([u8; 3], u8) {
    // Start at the Theme's 52% / 58%. Increase the veil only when even the
    // brightest/darkest ink cannot keep secondary text at 4.5:1. Channel bounds
    // conservatively cover every interpolated/blurred pixel, including gradients.
    let initial = if light { 133 } else { 148 };
    for alpha in initial..=255 {
        let backgrounds = bounds.map(|b| composite(veil, b, alpha as f32 / 255.0));
        let mut candidate = color;
        for step in 0..=100 {
            let value = if step == 100 {
                if light { [255; 3] } else { [0; 3] }
            } else {
                rgb(candidate)
            };
            // Include the opaque veil for the flat-background setting.
            if backgrounds.into_iter().chain([veil]).all(|background| {
                contrast(value, background) >= 4.5
                    && contrast(composite(value, background, 0.62), background) >= 4.55
            }) {
                return (value, alpha);
            }
            candidate[0] = (candidate[0] + if light { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        }
    }
    unreachable!("opaque veils admit accessible black/white secondary ink")
}

pub(crate) fn extract(image: &DynamicImage) -> Palette {
    let sample = image.resize_exact(48, 48, FilterType::Triangle).to_rgb8();
    let points: Vec<Lab> = sample.pixels().map(|p| lab(p.0)).collect();
    // Fixed first seed followed by farthest-point seeding: deterministic and no RNG.
    let mut centers = [points[0]; 5];
    for i in 1..5 {
        centers[i] = *points
            .iter()
            .max_by(|a, b| {
                let nearest = |p: Lab| {
                    centers[..i]
                        .iter()
                        .map(|c| distance(p, *c))
                        .fold(f32::INFINITY, f32::min)
                };
                nearest(**a).total_cmp(&nearest(**b))
            })
            .unwrap();
    }
    let mut counts = [0usize; 5];
    for _ in 0..12 {
        let mut sums = [[0.0; 3]; 5];
        counts = [0; 5];
        for &p in &points {
            let i = (0..5)
                .min_by(|&a, &b| distance(p, centers[a]).total_cmp(&distance(p, centers[b])))
                .unwrap();
            counts[i] += 1;
            for (sum, value) in sums[i].iter_mut().zip(p) {
                *sum += value;
            }
        }
        let previous = centers;
        for i in 0..5 {
            if counts[i] > 0 {
                centers[i] = sums[i].map(|v| v / counts[i] as f32);
            }
        }
        if (0..5).all(|i| distance(previous[i], centers[i]) < 0.000001) {
            break;
        }
    }
    let base_index = (0..5)
        .filter(|&i| counts[i] > 0 && chroma(centers[i]) >= 0.025)
        .max_by_key(|&i| counts[i])
        .or_else(|| (0..5).max_by_key(|&i| counts[i]))
        .unwrap();
    let base = centers[base_index];
    let vibrant = (0..5)
        .filter(|&i| counts[i] > 0 && centers[i][0] >= 0.45 && centers[i][0] <= 0.9)
        .max_by(|&a, &b| chroma(centers[a]).total_cmp(&chroma(centers[b])))
        .unwrap_or(base_index);
    let c = chroma(base);
    let mut glow = centers[vibrant];
    // Uniformly dark covers have no brighter cluster; keep its hue and lift the glow.
    glow[0] = glow[0].clamp(0.55, 0.85);
    let (a, b) = if c < 0.025 {
        (0.0, 0.0)
    } else {
        (base[1] / c, base[2] / c)
    };
    let tone = |l, c| [l, a * c, b * c];
    let vd = rgb(tone(0.28, 0.035));
    let vl = rgb(tone(0.92, 0.015));
    let mut bounds = [[255; 3], [0; 3]];
    for pixel in image.to_rgb8().pixels() {
        for (i, value) in pixel.0.into_iter().enumerate() {
            bounds[0][i] = bounds[0][i].min(value);
            bounds[1][i] = bounds[1][i].max(value);
        }
    }
    let (dark_ink, veil_dark_alpha) = ink(tone(0.97, 0.008), vd, bounds, true);
    let (light_ink, veil_light_alpha) = ink(tone(0.20, 0.025), vl, bounds, false);
    Palette {
        base: hex(rgb(if c < 0.025 { [base[0], 0.0, 0.0] } else { base })),
        glow: hex(rgb(if c < 0.025 { [0.7, 0.0, 0.0] } else { glow })),
        veil_dark: hex(vd),
        veil_light: hex(vl),
        ink_dark: hex(dark_ink),
        ink_light: hex(light_ink),
        veil_dark_alpha,
        veil_light_alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> [u8; 3] {
        [1, 3, 5].map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
    }
    #[test]
    fn composed_background_keeps_primary_and_secondary_accessible() {
        let mut images: Vec<_> = [[255; 3], [0; 3], [245, 250, 240], [5, 8, 12]]
            .into_iter()
            .map(|color| image::RgbImage::from_pixel(96, 96, image::Rgb(color)))
            .collect();
        images.push(image::RgbImage::from_fn(96, 96, |x, y| {
            image::Rgb([
                (x * 255 / 95) as u8,
                (y * 255 / 95) as u8,
                if x < 48 { 0 } else { 255 },
            ])
        }));
        for image in images {
            let palette = extract(&DynamicImage::ImageRgb8(image.clone()));
            for (veil, ink, alpha) in [
                (
                    &palette.veil_dark,
                    &palette.ink_dark,
                    palette.veil_dark_alpha,
                ),
                (
                    &palette.veil_light,
                    &palette.ink_light,
                    palette.veil_light_alpha,
                ),
            ] {
                let veil = parse(veil);
                let ink = parse(ink);
                for pixel in image.pixels() {
                    let background = composite(veil, pixel.0, alpha as f32 / 255.0);
                    for text_alpha in [1.0, 0.62, 0.74, 0.8] {
                        assert!(
                            contrast(composite(ink, background, text_alpha), background) >= 4.5
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn synthetic_palettes_have_accessible_inks() {
        for color in [[255, 0, 0], [128, 128, 128], [0, 0, 48]] {
            let img =
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb(color)));
            let palette = extract(&img);
            assert!(contrast(parse(&palette.ink_dark), parse(&palette.veil_dark)) >= 4.5);
            assert!(contrast(parse(&palette.ink_light), parse(&palette.veil_light)) >= 4.5);
            assert_eq!(palette, extract(&img));
            if color[0] == 128 {
                for s in [
                    &palette.base,
                    &palette.glow,
                    &palette.veil_dark,
                    &palette.veil_light,
                ] {
                    let c = parse(s);
                    assert_eq!(c[0], c[1]);
                    assert_eq!(c[1], c[2]);
                }
            }
        }
    }
}
