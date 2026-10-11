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

// While the background MOVES there is no local scrim, and the hypnotic colour lights
// float over the veil. Worst case for the text, per theme:
//   art (every dominant colour with weight >= MOVING_MIN_WEIGHT; slivers below that are
//   averaged away by the baked blur and the mirrored swirl; blends of two colours lie
//   between them) -> veil (alpha `a`, at most MOVING_VEIL_CAP) -> a colour light (alpha `b`).
// The lights are the cover's five colours with only their lightness moved into the theme's
// band, so the extremes are each colour at the band's lowest and highest lightness (keep the
// constants in sync with Theme.nowPlaying: blobLightDark/Light, blobSaturationLift,
// blobAlpha). Luminance is monotone in lightness and a blend of two colours lies between its
// ends, so checking those ends (and the light-free case b = 0) bounds every pixel.
// Search: the lowest `a` (from the static veil up to the cap) that keeps the lights at
// >= BLOB_KEEP of their full alpha; if none does, the cap with the strongest light alpha
// that still passes. QML clamps the lights to `b` only while moving. Returns (a, b), 0..=255.
const MOVING_MIN_WEIGHT: f32 = 0.05;
const MOVING_VEIL_CAP: u8 = 204; // 0.80: the veil stops here unless the art leaves no other way
const MOVING_VEIL_HARD_CAP: u8 = 230; // 0.90, Theme.nowPlaying.veilMovingMax
const BLOB_ALPHA_FULL: u8 = 217; // 0.85, Theme.nowPlaying.blobAlpha
const BLOB_KEEP: u8 = 130;
const BLOB_LIGHT_DARK: [f32; 2] = [0.22, 0.48];
const BLOB_LIGHT_LIGHT: [f32; 2] = [0.68, 0.9];
const BLOB_SATURATION_LIFT: f32 = 1.15;

fn rgb_to_hsl(c: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|v| f32::from(v) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let f = |n: f32| {
        let k = (n + h * 12.0).rem_euclid(12.0);
        (l - c / 2.0 + c * ((k - 3.0).min(9.0 - k).clamp(-1.0, 1.0) * 0.5 + 0.5)) * 255.0
    };
    [f(0.0), f(8.0), f(4.0)]
}
// Lightest and darkest light of each cover colour in the theme's band (hue untouched).
fn blob_lights(colors: &[[u8; 3]], dark: bool) -> Vec<[f32; 3]> {
    let band = if dark {
        BLOB_LIGHT_DARK
    } else {
        BLOB_LIGHT_LIGHT
    };
    colors
        .iter()
        .flat_map(|&c| {
            let (h, s, _) = rgb_to_hsl(c);
            band.map(|l| hsl_to_rgb(h, (s * BLOB_SATURATION_LIFT).min(1.0), l))
        })
        .collect()
}
fn moving_passes(ink: [f32; 3], bg: [f32; 3], dark: bool) -> bool {
    text_contrasts(ink, bg, dark).into_iter().all(|r| r >= 4.5)
}
fn moving_limits(
    veil: [u8; 3],
    colors: &[([u8; 3], f32)],
    start: u8,
    ink: [u8; 3],
    dark: bool,
) -> (u8, u8) {
    let veil = veil.map(f32::from);
    let ink = ink.map(f32::from);
    let art: Vec<[f32; 3]> = colors
        .iter()
        .filter(|(_, weight)| *weight >= MOVING_MIN_WEIGHT)
        .map(|(color, _)| color.map(f32::from))
        .collect();
    let lights = blob_lights(&colors.iter().map(|(c, _)| *c).collect::<Vec<_>>(), dark);
    let passes = |a: u8, b: u8| {
        let (a, b) = (f32::from(a) / 255.0, f32::from(b) / 255.0);
        art.iter().all(|&art| {
            let under = composite(veil, art, a);
            moving_passes(ink, under, dark)
                && lights
                    .iter()
                    .all(|&light| moving_passes(ink, composite(light, under, b), dark))
        })
    };
    let strongest = |a: u8| (0..=BLOB_ALPHA_FULL).rev().find(|&b| passes(a, b));
    for a in start.min(MOVING_VEIL_CAP)..=MOVING_VEIL_CAP {
        match strongest(a) {
            Some(b) if b >= BLOB_KEEP => return (a, b),
            Some(b) if a == MOVING_VEIL_CAP => return (a, b),
            _ => {}
        }
    }
    // Nearly white/black art: no white-or-black ink passes at the cap even with no lights (the
    // veil colour is fixed, so swapping the ink cannot help either). Go past the cap, only as
    // far as needed, to the hard ceiling; the lights are hidden while moving if needed.
    for a in MOVING_VEIL_CAP + 1..=MOVING_VEIL_HARD_CAP {
        if let Some(b) = strongest(a) {
            return (a, b);
        }
    }
    (MOVING_VEIL_HARD_CAP, 0)
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
    let mut populated: Vec<_> = (0..5).filter(|&i| counts[i] > 0).collect();
    populated.sort_by_key(|&i| std::cmp::Reverse(counts[i]));
    let mut colors: Vec<_> = populated
        .into_iter()
        .map(|i| (hex(rgb(centers[i])), counts[i] as f32 / points.len() as f32))
        .collect();
    // Monochrome artwork can leave empty clusters with unused seed colors.
    // Repeat the last populated color with zero weight, never invent a tone or
    // redistribute population: the five published weights still sum to one.
    let last_color = colors.last().unwrap().0.clone();
    colors.resize(5, (last_color, 0.0));
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
    let parsed: Vec<([u8; 3], f32)> = colors
        .iter()
        .map(|(color, weight)| {
            (
                [1, 3, 5].map(|i| u8::from_str_radix(&color[i..i + 2], 16).unwrap_or(0)),
                *weight,
            )
        })
        .collect();
    let (veil_dark_moving_alpha, blob_dark_moving_alpha) =
        moving_limits(vd, &parsed, veil_dark_alpha, dark_ink, true);
    let (veil_light_moving_alpha, blob_light_moving_alpha) =
        moving_limits(vl, &parsed, veil_light_alpha, light_ink, false);
    Palette {
        base: hex(rgb(if c < 0.025 { [base[0], 0.0, 0.0] } else { base })),
        glow: hex(rgb(if c < 0.025 { [0.7, 0.0, 0.0] } else { glow })),
        veil_dark: hex(vd),
        veil_light: hex(vl),
        ink_dark: hex(dark_ink),
        ink_light: hex(light_ink),
        colors,
        veil_dark_alpha,
        veil_light_alpha,
        veil_dark_moving_alpha,
        veil_light_moving_alpha,
        blob_dark_moving_alpha: Some(blob_dark_moving_alpha),
        blob_light_moving_alpha: Some(blob_light_moving_alpha),
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
    fn assert_clusters(palette: &Palette) {
        assert_eq!(palette.colors.len(), 5);
        for (color, weight) in &palette.colors {
            assert_eq!(color.len(), 7);
            assert!(color.starts_with('#'));
            parse(color);
            assert!(weight.is_finite() && (0.0..=1.0).contains(weight));
        }
        assert!(palette.colors.windows(2).all(|pair| pair[0].1 >= pair[1].1));
        let total: f32 = palette.colors.iter().map(|(_, weight)| weight).sum();
        assert!((total - 1.0).abs() < 0.00001, "population sum: {total}");
    }

    #[test]
    fn five_clusters_preserve_colors_and_population_order() {
        let colors = [
            [128; 3],
            [205, 165, 55],
            [180, 25, 40],
            [20, 60, 190],
            [25, 160, 60],
        ];
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(48, 48, |x, _| {
            let i = match x {
                0..24 => 0,
                24..36 => 1,
                36..42 => 2,
                42..46 => 3,
                _ => 4,
            };
            image::Rgb(colors[i])
        }));
        let palette = extract(&image);
        assert_clusters(&palette);
        for ((color, weight), (expected, width)) in palette
            .colors
            .iter()
            .zip(colors.into_iter().zip([24, 12, 6, 4, 2]))
        {
            assert_eq!(parse(color), expected);
            assert!((weight - width as f32 / 48.0).abs() < 0.00001);
        }
        assert_eq!(palette, extract(&image));
    }

    #[test]
    fn monochrome_clusters_repeat_last_color_with_zero_population() {
        for color in [[128; 3], [0; 3], [255; 3], [205, 165, 55]] {
            let image =
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb(color)));
            let palette = extract(&image);
            assert_clusters(&palette);
            assert_eq!(palette.colors[0], (hex(color), 1.0));
            assert!(
                palette.colors[1..]
                    .iter()
                    .all(|entry| entry == &(hex(color), 0.0))
            );
        }
        let almost = DynamicImage::ImageRgb8(image::RgbImage::from_fn(48, 48, |x, y| {
            image::Rgb(if x == 0 && y == 0 { [129; 3] } else { [128; 3] })
        }));
        let palette = extract(&almost);
        assert_clusters(&palette);
        let last = palette
            .colors
            .iter()
            .rposition(|(_, weight)| *weight > 0.0)
            .unwrap();
        assert!(last < 4);
        assert!(
            palette.colors[last + 1..]
                .iter()
                .all(|(color, weight)| color == &palette.colors[last].0 && *weight == 0.0)
        );
    }

    // HSV hue uses a circular distance; achromatic colors have no meaningful hue.
    fn hue_saturation(color: [u8; 3]) -> (f32, f32) {
        let [r, g, b] = color.map(|v| f32::from(v) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        if delta == 0.0 {
            return (0.0, 0.0);
        }
        let hue = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        (hue * 60.0, delta / max)
    }

    #[test]
    fn gray_and_gold_clusters_never_introduce_an_absent_hue() {
        let gray = [128; 3];
        let gold = [205, 165, 55];
        // The reduction interpolates the boundary, exercising minor hue drift too.
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(96, 96, |x, _| {
            image::Rgb(if x < 64 { gray } else { gold })
        }));
        let present: Vec<_> = [gray, gold]
            .into_iter()
            .map(hue_saturation)
            .filter(|(_, saturation)| *saturation >= 0.05)
            .map(|(hue, _)| hue)
            .collect();
        let palette = extract(&image);
        assert_clusters(&palette);
        assert!(
            palette
                .colors
                .iter()
                .any(|(color, _)| hue_saturation(parse(color)).1 < 0.05)
        );
        assert!(
            palette
                .colors
                .iter()
                .any(|(color, _)| hue_saturation(parse(color)).1 > 0.5)
        );
        for (color, _) in &palette.colors {
            let (hue, saturation) = hue_saturation(parse(color));
            if saturation < 0.05 {
                continue;
            }
            assert!(
                present.iter().any(|present| {
                    let delta = (hue - present).abs();
                    delta.min(360.0 - delta) <= 8.0
                }),
                "absent hue in {color}: {hue}"
            );
        }
    }

    #[tokio::test]
    async fn legacy_palette_without_clusters_deserializes_and_rebuilds_offline() {
        use velo_domain::CoverArtService;
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

        let server = MockServer::start().await;
        Mock::given(path("/legacy-cover"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let key = format!("{}/legacy-cover", server.uri());
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            64,
            image::Rgb([205, 165, 55]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        for blur in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let cache = crate::covers::CoverArtCache::with_cache_dir(dir.path()).unwrap();
            let first = cache
                .process_bytes_with_blur(&key, bytes.get_ref().clone(), blur)
                .await
                .unwrap();
            let palette_path = first.cover.with_extension("json");
            let mut old = serde_json::to_value(&first.palette).unwrap();
            old.as_object_mut().unwrap().remove("colors");
            for key in [
                "veil_dark_moving_alpha",
                "veil_light_moving_alpha",
                "blob_dark_moving_alpha",
                "blob_light_moving_alpha",
            ] {
                old.as_object_mut().unwrap().remove(key);
            }
            old["base"] = "#012345".into();
            let old = serde_json::to_vec(&old).unwrap();
            let legacy: Palette = serde_json::from_slice(&old).unwrap();
            assert!(legacy.colors.is_empty());
            crate::atomic_write(&palette_path, &old).unwrap();

            let rebuilt = cache.process_with_blur(&key, blur).await.unwrap();
            assert_eq!(rebuilt.cover, first.cover);
            assert_ne!(rebuilt.palette.base, legacy.base);
            assert_clusters(&rebuilt.palette);
            assert!(rebuilt.palette.veil_dark_moving_alpha >= rebuilt.palette.veil_dark_alpha);
            assert!(rebuilt.palette.veil_light_moving_alpha > 0);
            let saved: Palette =
                serde_json::from_slice(&std::fs::read(palette_path).unwrap()).unwrap();
            assert_eq!(saved, rebuilt.palette);
            // Subsequent requests reuse the migrated palette without an upgrade.
            assert_eq!(rebuilt, cache.process_with_blur(&key, blur).await.unwrap());
            assert_eq!(rebuilt.palette, cache.process(&key).await.unwrap().palette);
        }
        server.verify().await;
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

    // While the background moves there is no scrim and the colour lights float over the
    // veil: ink >= 4.5:1 over art -> capped veil -> (alpha-limited) light, for every
    // dominant colour and every light colour of the cover, and the veil never passes the cap.
    #[test]
    fn moving_veil_keeps_ink_accessible_over_the_worst_cluster() {
        let halves = |l: [u8; 3], r: [u8; 3]| {
            image::RgbImage::from_fn(96, 96, |x, _| image::Rgb(if x < 48 { l } else { r }))
        };
        let images = vec![
            image::RgbImage::from_pixel(96, 96, image::Rgb([250, 250, 250])),
            image::RgbImage::from_pixel(96, 96, image::Rgb([4, 4, 6])),
            image::RgbImage::from_pixel(96, 96, image::Rgb([128; 3])),
            halves([255; 3], [0; 3]),
            halves([240, 200, 40], [20, 40, 160]),
            dark_blue_brown_cover().to_rgb8(),
            image::RgbImage::from_fn(96, 96, |x, y| {
                image::Rgb([
                    (x * 255 / 95) as u8,
                    (y * 255 / 95) as u8,
                    (x * y % 256) as u8,
                ])
            }),
        ];
        for (index, image) in images.into_iter().enumerate() {
            let palette = extract(&DynamicImage::ImageRgb8(image));
            let cover: Vec<[u8; 3]> = palette.colors.iter().map(|(c, _)| parse(c)).collect();
            for (veil, ink, alpha, blob, start, dark) in [
                (
                    &palette.veil_dark,
                    &palette.ink_dark,
                    palette.veil_dark_moving_alpha,
                    palette.blob_dark_moving_alpha.unwrap(),
                    palette.veil_dark_alpha,
                    true,
                ),
                (
                    &palette.veil_light,
                    &palette.ink_light,
                    palette.veil_light_moving_alpha,
                    palette.blob_light_moving_alpha.unwrap(),
                    palette.veil_light_alpha,
                    false,
                ),
            ] {
                assert!(alpha >= start.min(MOVING_VEIL_CAP), "cover {index}");
                assert!(alpha <= MOVING_VEIL_HARD_CAP, "cover {index}: veil {alpha}");
                assert!(blob <= BLOB_ALPHA_FULL);
                let veil = parse(veil).map(f32::from);
                let ink = parse(ink).map(f32::from);
                let (a, b) = (f32::from(alpha) / 255.0, f32::from(blob) / 255.0);
                let lights = blob_lights(&cover, dark);
                assert_eq!(lights.len(), 10);
                println!("cover {index} dark={dark}: veil {alpha}/255, lights {blob}/255");
                for (color, weight) in &palette.colors {
                    if *weight < MOVING_MIN_WEIGHT {
                        continue;
                    }
                    let under = composite(veil, parse(color).map(f32::from), a);
                    let mut backgrounds = vec![under];
                    backgrounds.extend(lights.iter().map(|&light| composite(light, under, b)));
                    for background in backgrounds {
                        for (role, ratio) in
                            qml_contrasts(ink, background, dark).into_iter().enumerate()
                        {
                            assert!(
                                ratio >= 4.5,
                                "cover {index}, dark={dark}, role {role}: {ratio}"
                            );
                        }
                    }
                }
            }
        }
    }
}
