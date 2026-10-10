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
fn luminance(rgb: [f32; 3]) -> f32 {
    let [r, g, b] = rgb.map(|v| linear(v / 255.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}
fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn composite(foreground: [f32; 3], background: [f32; 3], alpha: f32) -> [f32; 3] {
    std::array::from_fn(|i| foreground[i] * alpha + background[i] * (1.0 - alpha))
}

fn text_contrasts(ink: [f32; 3], background: [f32; 3], dark: bool) -> [f32; 4] {
    // Theme.qml: secondary text (including the title-bar timer), inactive lyrics,
    // and the active lyric's pill. Alpha blending happens in sRGB before WCAG luma.
    let (secondary, inactive, pill) = if dark {
        (0.62, 0.74, 0.12)
    } else {
        (0.80, 0.80, 0.10)
    };
    [
        contrast(ink, background),
        contrast(composite(ink, background, secondary), background),
        contrast(composite(ink, background, inactive), background),
        contrast(ink, composite(ink, background, pill)),
    ]
}

fn scrim(veil: [u8; 3], bounds: [[u8; 3]; 2], global: u8, ink: [u8; 3], dark: bool) -> u8 {
    // White on the deep shadow / black on the light tint maximize contrast and
    // minimize the local opacity. Channel bounds include all blurred/interpolated
    // pixels; calculate in sRGB without intermediate rounding, just like QML.
    let veil = veil.map(f32::from);
    let ink = ink.map(f32::from);
    let global = f32::from(global) / 255.0;
    // For black/white ink at these alphas, the 4.5:1 luminance inequality has
    // its channel minima at interval endpoints. Check all eight corners, since
    // translucent ink can have a worse corner than just the RGB min/max pair.
    let corners: [[f32; 3]; 8] = std::array::from_fn(|corner| {
        std::array::from_fn(|i| f32::from(bounds[(corner >> i) & 1][i]))
    });
    (0..=255)
        .find(|&local| {
            let alpha = 1.0 - (1.0 - global) * (1.0 - f32::from(local) / 255.0);
            corners
                .map(|b| composite(veil, b, alpha))
                .into_iter()
                .chain([veil]) // Also support the flat-background setting.
                .all(|background| {
                    text_contrasts(ink, background, dark)
                        .into_iter()
                        .all(|ratio| ratio >= 4.5)
                })
        })
        .expect("opaque veils admit accessible black/white ink at the QML alphas")
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
    // Retain the dominant hue, with enough chroma to read as album color rather
    // than a neutral gray wash. Achromatic artwork stays achromatic.
    let vd = rgb(tone(0.26, 0.09));
    let vl = rgb(tone(0.82, 0.13));
    let mut bounds = [[255; 3], [0; 3]];
    for pixel in image.to_rgb8().pixels() {
        for (i, value) in pixel.0.into_iter().enumerate() {
            bounds[0][i] = bounds[0][i].min(value);
            bounds[1][i] = bounds[1][i].max(value);
        }
    }
    // 77..140 / 255 = 0.302..0.549. Brighter artwork needs more dark veil;
    // darker artwork needs more light tint. Contrast never raises this global veil.
    let brightness = luminance(rgb(base).map(f32::from));
    let veil_dark_alpha = 77 + (63.0 * brightness).round() as u8;
    let veil_light_alpha = 77 + (63.0 * (1.0 - brightness)).round() as u8;
    let dark_ink = [255; 3];
    let light_ink = [0; 3];
    let scrim_dark_alpha = scrim(vd, bounds, veil_dark_alpha, dark_ink, true);
    let scrim_light_alpha = scrim(vl, bounds, veil_light_alpha, light_ink, false);
    Palette {
        base: hex(rgb(if c < 0.025 { [base[0], 0.0, 0.0] } else { base })),
        glow: hex(rgb(if c < 0.025 { [0.7, 0.0, 0.0] } else { glow })),
        veil_dark: hex(vd),
        veil_light: hex(vl),
        ink_dark: hex(dark_ink),
        ink_light: hex(light_ink),
        veil_dark_alpha,
        veil_light_alpha,
        scrim_dark_alpha,
        scrim_light_alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> [u8; 3] {
        [1, 3, 5].map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
    }
    // Keep the QML alphas explicit in tests so a production change cannot silently
    // weaken the four checks. The pill is another layer beneath opaque active text.
    fn qml_contrasts(ink: [f32; 3], background: [f32; 3], dark: bool) -> [f32; 4] {
        let [secondary, inactive, pill] = if dark {
            [0.62, 0.74, 0.12]
        } else {
            [0.80, 0.80, 0.10]
        };
        [
            contrast(ink, background),
            contrast(composite(ink, background, secondary), background),
            contrast(composite(ink, background, inactive), background),
            contrast(ink, composite(ink, background, pill)),
        ]
    }
    #[test]
    fn veil_and_scrim_keep_ink_accessible_over_every_pixel() {
        let mut images: Vec<_> = [
            [255; 3],
            [0; 3],
            [245, 250, 240],
            [5, 8, 12],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
        ]
        .into_iter()
        .map(|color| image::RgbImage::from_pixel(96, 96, image::Rgb(color)))
        .collect();
        images.push(image::RgbImage::from_fn(96, 96, |x, _| {
            image::Rgb(if x < 48 { [255; 3] } else { [0; 3] })
        }));
        images.push(dark_blue_brown_cover().to_rgb8());
        images.push(image::RgbImage::from_fn(96, 96, |x, y| {
            image::Rgb([
                (x * 255 / 95) as u8,
                (y * 255 / 95) as u8,
                if x < 48 { 0 } else { 255 },
            ])
        }));
        for (index, image) in images.into_iter().enumerate() {
            let palette = extract(&DynamicImage::ImageRgb8(image.clone()));
            println!(
                "cover {index}: veil dark/light={:.3}/{:.3}, scrim dark/light={:.3}/{:.3}",
                f32::from(palette.veil_dark_alpha) / 255.0,
                f32::from(palette.veil_light_alpha) / 255.0,
                f32::from(palette.scrim_dark_alpha) / 255.0,
                f32::from(palette.scrim_light_alpha) / 255.0,
            );
            for (veil, ink, global, local, dark) in [
                (
                    &palette.veil_dark,
                    &palette.ink_dark,
                    palette.veil_dark_alpha,
                    palette.scrim_dark_alpha,
                    true,
                ),
                (
                    &palette.veil_light,
                    &palette.ink_light,
                    palette.veil_light_alpha,
                    palette.scrim_light_alpha,
                    false,
                ),
            ] {
                let veil = parse(veil).map(f32::from);
                let ink = parse(ink).map(f32::from);
                let global = f32::from(global) / 255.0;
                let local = f32::from(local) / 255.0;
                assert!((0.30..=0.55).contains(&global));
                let mut bounds = [[255; 3], [0; 3]];
                for pixel in image.pixels() {
                    for (i, value) in pixel.0.into_iter().enumerate() {
                        bounds[0][i] = bounds[0][i].min(value);
                        bounds[1][i] = bounds[1][i].max(value);
                    }
                }
                // Include channel corners and interpolated pixels: blur/scaling may
                // create colors absent from the original artwork.
                let corners: [[u8; 3]; 8] = std::array::from_fn(|corner| {
                    std::array::from_fn(|i| bounds[(corner >> i) & 1][i])
                });
                let blurred = image::imageops::blur(&image, 3.0);
                let mut worst = [f32::INFINITY; 4];
                for pixel in image
                    .pixels()
                    .chain(blurred.pixels())
                    .map(|pixel| pixel.0)
                    .chain(corners)
                {
                    // Exercise QML's two separate layers, not just the combined alpha.
                    let background =
                        composite(veil, composite(veil, pixel.map(f32::from), global), local);
                    for (minimum, ratio) in
                        worst.iter_mut().zip(qml_contrasts(ink, background, dark))
                    {
                        *minimum = minimum.min(ratio);
                    }
                }
                for (role, ratio) in ["opaque", "secondary", "inactive", "pill"]
                    .into_iter()
                    .zip(worst)
                {
                    assert!(ratio >= 4.5, "cover {index}, dark={dark}, {role}: {ratio}");
                }
                assert!(
                    qml_contrasts(ink, veil, dark)
                        .into_iter()
                        .all(|ratio| ratio >= 4.5)
                );
            }
        }
    }
    #[test]
    fn synthetic_palettes_have_accessible_inks() {
        for color in [[255, 0, 0], [128, 128, 128], [0, 0, 48]] {
            let img =
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb(color)));
            let palette = extract(&img);
            assert!(
                contrast(
                    parse(&palette.ink_dark).map(f32::from),
                    parse(&palette.veil_dark).map(f32::from)
                ) >= 4.5
            );
            assert!(
                contrast(
                    parse(&palette.ink_light).map(f32::from),
                    parse(&palette.veil_light).map(f32::from)
                ) >= 4.5
            );
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

    fn dark_blue_brown_cover() -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(96, 96, |x, y| {
            image::Rgb(if x < 64 {
                [10 + (y / 8) as u8, 25, 60]
            } else {
                [65, 35 + (y / 8) as u8, 18]
            })
        }))
    }

    #[test]
    fn dark_album_keeps_saturated_veils_and_visible_art_in_both_modes() {
        let palette = extract(&dark_blue_brown_cover());
        for (veil, alpha) in [
            (&palette.veil_dark, palette.veil_dark_alpha),
            (&palette.veil_light, palette.veil_light_alpha),
        ] {
            let channels = parse(veil).map(|v| f32::from(v) / 255.0);
            let max = channels.into_iter().fold(0.0, f32::max);
            let min = channels.into_iter().fold(1.0, f32::min);
            let saturation = (max - min) / (1.0 - (max + min - 1.0).abs());
            assert!(saturation > 0.25, "{veil}: HSL saturation {saturation}");
            assert!(f32::from(alpha) / 255.0 <= 0.55);
        }
        println!("dark blue/brown: {palette:?}");
    }

    #[test]
    fn scrim_is_minimal_at_published_alpha_precision() {
        let bounds = [[0; 3], [255; 3]];
        for (veil, global, ink, dark) in [
            ([12, 25, 55], 77, [255; 3], true),
            ([165, 200, 235], 140, [0; 3], false),
        ] {
            let local = scrim(veil, bounds, global, ink, dark);
            assert!(local > 0);
            let worst = |local: u8| {
                let alpha =
                    1.0 - (1.0 - f32::from(global) / 255.0) * (1.0 - f32::from(local) / 255.0);
                (0..8)
                    .flat_map(|corner| {
                        let pixel: [u8; 3] = std::array::from_fn(|i| bounds[(corner >> i) & 1][i]);
                        qml_contrasts(
                            ink.map(f32::from),
                            composite(veil.map(f32::from), pixel.map(f32::from), alpha),
                            dark,
                        )
                    })
                    .fold(f32::INFINITY, f32::min)
            };
            assert!(worst(local) >= 4.5);
            assert!(worst(local - 1) < 4.5);
        }
    }
}
