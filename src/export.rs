use crate::clean::{Paper, encode_png};
use flate2::{Compression, write::ZlibEncoder};
use image::GrayImage;
use std::io::Write;

pub fn pdf(pages: &[&GrayImage], paper: Paper) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("there are no pages to save".into());
    }
    let (pw, ph) = paper.points();
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let page_ids: Vec<usize> = (0..pages.len()).map(|i| 3 + i * 3).collect();
    let kids: Vec<String> = page_ids.iter().map(|id| format!("{id} 0 R")).collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    for (i, img) in pages.iter().enumerate() {
        let id = 3 + i * 3;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                id + 1,
                id + 2
            )
            .into_bytes(),
        );
        let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
        z.write_all(img.as_raw()).map_err(|e| e.to_string())?;
        let data = z.finish().map_err(|e| e.to_string())?;
        let mut image = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            img.width(),
            img.height(),
            data.len()
        )
        .into_bytes();
        image.extend_from_slice(&data);
        image.extend_from_slice(b"\nendstream");
        objects.push(image);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes());
    }
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    Ok(out)
}

pub fn png(img: &GrayImage) -> Result<Vec<u8>, String> {
    encode_png(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn page(paper: Paper) -> GrayImage {
        let (w, h) = paper.pixels();
        let mut img = GrayImage::from_pixel(w, h, Luma([255]));
        for x in 100..400 {
            img.put_pixel(x, 200, Luma([0]));
        }
        img
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn a_pdf_has_one_page_per_image_at_paper_size() {
        let a = page(Paper::A4);
        let doc = pdf(&[&a, &a, &a], Paper::A4).unwrap();
        assert!(doc.starts_with(b"%PDF-1.4"));
        assert!(doc.ends_with(b"%%EOF\n"));
        assert!(find(&doc, b"/Count 3").is_some());
        assert!(find(&doc, b"/MediaBox [0 0 595.30 841.90]").is_some());
        let letter = pdf(&[&page(Paper::Letter)], Paper::Letter).unwrap();
        assert!(find(&letter, b"/MediaBox [0 0 612.00 792.00]").is_some());
    }

    #[test]
    fn the_xref_offsets_point_at_their_objects() {
        let a = page(Paper::A4);
        let doc = pdf(&[&a, &a], Paper::A4).unwrap();
        let text = String::from_utf8_lossy(&doc).to_string();
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(doc[start..].starts_with(b"xref"));
        let table = &text[start..];
        let entries: Vec<usize> = table
            .lines()
            .filter(|l| l.ends_with(" 00000 n "))
            .map(|l| l[..10].parse().unwrap())
            .collect();
        assert_eq!(entries.len(), 1 + 1 + 2 * 3 - 0);
        for (i, off) in entries.iter().enumerate() {
            let want = format!("{} 0 obj", i + 1);
            assert!(doc[*off..].starts_with(want.as_bytes()), "object {}", i + 1);
        }
    }

    #[test]
    fn the_image_stream_inflates_back_to_the_page() {
        use std::io::Read;
        let a = page(Paper::A4);
        let doc = pdf(&[&a], Paper::A4).unwrap();
        let head = find(&doc, b"/Filter /FlateDecode /Length ").unwrap();
        let len_start = head + b"/Filter /FlateDecode /Length ".len();
        let len_end = len_start + doc[len_start..].iter().position(|b| *b == b' ').unwrap();
        let len: usize = std::str::from_utf8(&doc[len_start..len_end]).unwrap().parse().unwrap();
        let data_start = len_end + find(&doc[len_end..], b"stream\n").unwrap() + 7;
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&doc[data_start..data_start + len]).read_to_end(&mut raw).unwrap();
        assert_eq!(raw, a.as_raw().as_slice());
    }

    #[test]
    fn no_pages_is_an_error() {
        assert!(pdf(&[], Paper::A4).is_err());
    }

    #[test]
    fn a_png_round_trips() {
        let a = page(Paper::Letter);
        let back = image::load_from_memory(&png(&a).unwrap()).unwrap().to_luma8();
        assert_eq!(back, a);
    }
}
