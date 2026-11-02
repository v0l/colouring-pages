use image::{GrayImage, Luma, imageops::FilterType};

pub const DPI: u32 = 200;
const MARGIN_INCH: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Paper {
    #[default]
    A4,
    Letter,
}

impl Paper {
    pub fn id(self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::Letter => "letter",
        }
    }

    pub fn from_id(id: &str) -> Paper {
        if id.eq_ignore_ascii_case("letter") { Paper::Letter } else { Paper::A4 }
    }

    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::Letter => "letter",
        }
    }

    pub fn inches(self) -> (f32, f32) {
        match self {
            Paper::A4 => (8.268, 11.693),
            Paper::Letter => (8.5, 11.0),
        }
    }

    pub fn pixels(self) -> (u32, u32) {
        let (w, h) = self.inches();
        ((w * DPI as f32).round() as u32, (h * DPI as f32).round() as u32)
    }

    pub fn points(self) -> (f32, f32) {
        let (w, h) = self.inches();
        (w * 72.0, h * 72.0)
    }
}

pub fn otsu(gray: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum: f64 = hist.iter().enumerate().map(|(i, c)| i as f64 * *c as f64).sum();
    let (mut below, mut weight_below, mut best, mut best_t) = (0.0f64, 0u64, -1.0f64, 128u8);
    for (t, count) in hist.iter().enumerate() {
        weight_below += count;
        if weight_below == 0 {
            continue;
        }
        let weight_above = total - weight_below;
        if weight_above == 0 {
            break;
        }
        below += t as f64 * *count as f64;
        let mean_below = below / weight_below as f64;
        let mean_above = (sum - below) / weight_above as f64;
        let between = weight_below as f64 * weight_above as f64 * (mean_below - mean_above).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t.clamp(60, 200)
}

pub fn threshold(gray: &GrayImage, level: u8) -> GrayImage {
    let mut out = gray.clone();
    for p in out.pixels_mut() {
        p.0[0] = if p.0[0] > level { 255 } else { 0 };
    }
    out
}

fn remove_regions(img: &mut GrayImage, ink: bool, smaller_than: usize) {
    let (w, h) = img.dimensions();
    let target = if ink { 0u8 } else { 255u8 };
    let flip = 255 - target;
    let mut seen = vec![false; (w * h) as usize];
    let mut stack = Vec::new();
    let mut region = Vec::new();
    for start in 0..(w * h) as usize {
        if seen[start] || img.as_raw()[start] != target {
            continue;
        }
        stack.push(start);
        seen[start] = true;
        region.clear();
        let mut touches_edge = false;
        while let Some(i) = stack.pop() {
            region.push(i);
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                touches_edge = true;
            }
            let mut push = |nx: i64, ny: i64| {
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return;
                }
                let j = (ny as u32 * w + nx as u32) as usize;
                if !seen[j] && img.as_raw()[j] == target {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                push(x as i64 + dx, y as i64 + dy);
            }
        }
        if region.len() < smaller_than && (ink || !touches_edge) {
            for &i in &region {
                img.put_pixel((i as u32) % w, (i as u32) / w, Luma([flip]));
            }
        }
    }
}

pub fn clean(source: &GrayImage) -> GrayImage {
    let mut img = threshold(source, otsu(source));
    let area = (img.width() * img.height()) as f32;
    let speck = ((area / 1_000_000.0) * 30.0).max(8.0) as usize;
    remove_regions(&mut img, true, speck);
    remove_regions(&mut img, false, (speck / 3).max(4));
    img
}

pub fn fit(clean: &GrayImage, paper: Paper) -> GrayImage {
    let (pw, ph) = paper.pixels();
    let margin = (MARGIN_INCH * DPI as f32).round() as u32;
    let (aw, ah) = (pw - 2 * margin, ph - 2 * margin);
    let (w, h) = clean.dimensions();
    let scale = (aw as f32 / w as f32).min(ah as f32 / h as f32);
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    let resized = image::imageops::resize(clean, nw, nh, FilterType::Lanczos3);
    let resized = threshold(&resized, 128);
    let mut page = GrayImage::from_pixel(pw, ph, Luma([255]));
    image::imageops::replace(&mut page, &resized, ((pw - nw) / 2) as i64, ((ph - nh) / 2) as i64);
    page
}

pub fn make_page(png: &[u8], paper: Paper) -> Result<GrayImage, String> {
    let decoded = image::load_from_memory(png).map_err(|e| format!("the picture could not be read: {e}"))?;
    let gray = decoded.to_luma8();
    if gray.width() < 8 || gray.height() < 8 {
        return Err("the picture was too small".into());
    }
    Ok(fit(&clean(&gray), paper))
}

pub fn is_black_and_white(img: &GrayImage) -> bool {
    img.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255)
}

pub fn ink_ratio(img: &GrayImage) -> f32 {
    let black = img.pixels().filter(|p| p.0[0] == 0).count();
    black as f32 / (img.width() * img.height()).max(1) as f32
}

pub fn encode_png(img: &GrayImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey_test_image() -> GrayImage {
        let mut img = GrayImage::from_pixel(400, 500, Luma([235]));
        for y in 0..500u32 {
            for x in 0..400u32 {
                let d = (((x as f32 - 200.0).powi(2) + (y as f32 - 250.0).powi(2)).sqrt() - 120.0).abs();
                if d < 3.0 {
                    img.put_pixel(x, y, Luma([20]));
                } else if d < 6.0 {
                    img.put_pixel(x, y, Luma([128]));
                } else if (x / 20 + y / 20) % 3 == 0 && d > 40.0 {
                    img.put_pixel(x, y, Luma([200 + ((x + y) % 40) as u8]));
                }
            }
        }
        img
    }

    #[test]
    fn a_grey_image_becomes_only_zero_and_255() {
        let grey = grey_test_image();
        assert!(!is_black_and_white(&grey));
        let out = clean(&grey);
        assert!(is_black_and_white(&out));
        let ink = ink_ratio(&out);
        assert!(ink > 0.01 && ink < 0.2, "the circle survives: {ink}");
    }

    #[test]
    fn every_pixel_of_a_fitted_page_is_zero_or_255() {
        for paper in [Paper::A4, Paper::Letter] {
            let page = fit(&clean(&grey_test_image()), paper);
            assert!(is_black_and_white(&page), "{paper:?}");
        }
    }

    #[test]
    fn a_fitted_page_is_exactly_paper_sized() {
        for (paper, want) in [(Paper::A4, (1654, 2339)), (Paper::Letter, (1700, 2200))] {
            for (w, h) in [(400, 500), (1000, 700), (64, 2048)] {
                let page = fit(&GrayImage::from_pixel(w, h, Luma([255])), paper);
                assert_eq!(page.dimensions(), want);
            }
        }
    }

    #[test]
    fn the_art_stays_inside_the_margin_and_keeps_its_aspect() {
        let mut art = GrayImage::from_pixel(300, 300, Luma([255]));
        for i in 0..300 {
            art.put_pixel(i, 0, Luma([0]));
            art.put_pixel(i, 299, Luma([0]));
            art.put_pixel(0, i, Luma([0]));
            art.put_pixel(299, i, Luma([0]));
        }
        let page = fit(&art, Paper::A4);
        let margin = (MARGIN_INCH * DPI as f32) as u32;
        let (mut minx, mut maxx, mut miny, mut maxy) = (u32::MAX, 0, u32::MAX, 0);
        for (x, y, p) in page.enumerate_pixels() {
            if p.0[0] == 0 {
                minx = minx.min(x);
                maxx = maxx.max(x);
                miny = miny.min(y);
                maxy = maxy.max(y);
            }
        }
        assert!(minx >= margin - 2 && maxx <= page.width() - margin + 2);
        assert!(miny >= margin && maxy <= page.height() - margin);
        let (aw, ah) = (maxx - minx, maxy - miny);
        assert!((aw as i32 - ah as i32).abs() <= 3, "square stays square: {aw}x{ah}");
        let centre_x = (minx + maxx) / 2;
        assert!((centre_x as i32 - page.width() as i32 / 2).abs() <= 2);
    }

    #[test]
    fn specks_are_removed_and_lines_stay() {
        let mut img = GrayImage::from_pixel(600, 600, Luma([255]));
        for x in 100..500 {
            for y in 300..306 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
        for (x, y) in [(50, 50), (51, 50), (50, 51), (550, 100)] {
            img.put_pixel(x, y, Luma([0]));
        }
        let out = clean(&img);
        assert_eq!(out.get_pixel(50, 50).0[0], 255);
        assert_eq!(out.get_pixel(550, 100).0[0], 255);
        assert_eq!(out.get_pixel(300, 302).0[0], 0);
    }

    #[test]
    fn pinholes_in_a_line_are_filled_but_the_background_is_not_eaten() {
        let mut img = GrayImage::from_pixel(600, 600, Luma([255]));
        for x in 100..500 {
            for y in 300..312 {
                img.put_pixel(x, y, Luma([0]));
            }
        }
        img.put_pixel(300, 305, Luma([255]));
        let out = clean(&img);
        assert_eq!(out.get_pixel(300, 305).0[0], 0);
        assert_eq!(out.get_pixel(5, 5).0[0], 255);
    }

    #[test]
    fn otsu_splits_a_two_level_image() {
        let mut img = GrayImage::from_pixel(100, 100, Luma([240]));
        for x in 0..30 {
            for y in 0..100 {
                img.put_pixel(x, y, Luma([30]));
            }
        }
        let t = otsu(&img);
        assert!(t > 30 && t < 240);
    }

    #[test]
    fn a_blank_page_stays_blank() {
        let out = clean(&GrayImage::from_pixel(100, 100, Luma([250])));
        assert!(out.pixels().all(|p| p.0[0] == 255));
    }

    #[test]
    fn make_page_reads_a_png_and_rejects_junk() {
        let png = encode_png(&grey_test_image()).unwrap();
        let page = make_page(&png, Paper::Letter).unwrap();
        assert_eq!(page.dimensions(), Paper::Letter.pixels());
        assert!(is_black_and_white(&page));
        assert!(make_page(b"not a picture", Paper::A4).is_err());
    }
}
