//! PNG → the client's palettised sprite format.
//!
//! Per image the shared `index` gets `p2 tileW, p2 tileH, p1 colours,
//! p3 colour…` and per tile `p1 cropX, p1 cropY, p2 w, p2 h, p1 order`; the
//! image's `data` starts with `p2 index.pos` and holds one palette index per
//! pixel. Magenta (0xFF00FF) is transparent: palette slot 0 and excluded from
//! the crop bounds. Alpha is ignored. More than 255 colours is an error (no
//! content PNG reaches it).

use crate::io::packet::Packet;
use crate::pack::parse::int;

/// An RGBA8 image.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Image {
    pub fn load(path: &str) -> Result<Image, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(|e| format!("{path}: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| format!("{path}: image too large"))?];
        let info = reader.next_frame(&mut buf).map_err(|e| format!("{path}: {e}"))?;
        let (w, h) = (info.width as usize, info.height as usize);
        let px = &buf[..info.buffer_size()];
        let data: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => px.to_vec(),
            png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
            png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return Err(format!("{path}: palette not expanded")),
        };
        Ok(Image { width: w, height: h, data })
    }

    fn rgb(&self, x: usize, y: usize) -> u32 {
        let p = (x + y * self.width) * 4;
        ((self.data[p] as u32) << 16) | ((self.data[p + 1] as u32) << 8) | self.data[p + 2] as u32
    }

    /// `clone().crop({x, y, w, h})`.
    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        let mut data = Vec::with_capacity(w * h * 4);
        for row in y..y + h {
            let p = (x + row * self.width) * 4;
            data.extend_from_slice(&self.data[p..p + w * 4]);
        }
        Image { width: w, height: h, data }
    }
}

const MAGENTA: u32 = 0xff00ff;

/// `getPixelBounds`: the box of non-magenta pixels (the whole image if none).
fn bounds(img: &Image) -> (usize, usize, usize, usize) {
    let (mut left, mut top, mut right, mut bottom) = (img.width, img.height, -1isize, -1isize);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.rgb(x, y) == MAGENTA {
                continue;
            }
            left = left.min(x);
            top = top.min(y);
            right = right.max(x as isize);
            bottom = bottom.max(y as isize);
        }
    }
    if right == -1 { (0, 0, img.width, img.height) } else { (left, top, right as usize - left + 1, bottom as usize - top + 1) }
}

/// `generatePixelOrder`: 1 (column-major) when columns change colour less often.
fn pixel_order(img: &Image, left: usize, top: usize, width: usize, height: usize) -> u8 {
    let (mut rows, mut columns) = (0, 0);
    let (mut prev_row, mut prev_col) = (None, None);
    for i in 0..width * height {
        let row = img.rgb(left + i % width, top + i / width);
        let col = img.rgb(left + i / height, top + i % height);
        if i > 0 {
            rows += (Some(row) != prev_row) as usize;
            columns += (Some(col) != prev_col) as usize;
        }
        prev_row = Some(row);
        prev_col = Some(col);
    }
    (columns < rows) as u8
}

fn write_image(img: &Image, data: &mut Packet, index: &mut Packet, colors: &[u32]) {
    let (left, top, width, height) = bounds(img);
    let order = pixel_order(img, left, top, width, height);
    index.p1(left as i64);
    index.p1(top as i64);
    index.p2(width as i64);
    index.p2(height as i64);
    index.p1(order as i64);
    let (outer, inner) = if order == 1 { (width, height) } else { (height, width) };
    for a in 0..outer {
        for b in 0..inner {
            let (x, y) = if order == 1 { (a, b) } else { (b, a) };
            let rgb = img.rgb(x + left, y + top);
            data.p1(colors.iter().position(|&c| c == rgb).map_or(-1, |i| i as i64));
        }
    }
}

/// `generatePalette`: magenta first, then colours in first-seen order.
fn palette(img: &Image) -> Vec<u32> {
    let mut colors = vec![MAGENTA];
    for j in 0..img.width * img.height {
        let rgb = img.rgb(j % img.width, j / img.width);
        if rgb != MAGENTA && !colors.contains(&rgb) {
            colors.push(rgb);
        }
    }
    colors
}

/// `convertImage(index, srcPath, safeName)`: the image's `data` packet.
pub fn convert_image(index: &mut Packet, src_path: &str, safe_name: &str) -> Result<Packet, String> {
    let mut data = Packet::alloc(4);
    data.p2(index.pos as i64);
    let img = Image::load(&format!("{src_path}/{safe_name}.png"))?;

    let (mut tile_x, mut tile_y): (usize, usize) = (img.width, img.height);
    let meta = format!("{src_path}/meta/{safe_name}.opt");
    if std::path::Path::new(&meta).exists() {
        let text = crate::pack::name_map::read_ascii(&meta).map_err(|e| format!("{meta}: {e}"))?;
        let first = text.split('\n').next().unwrap_or("");
        let first = first.strip_suffix('\r').unwrap_or(first);
        // `<tile width>x<tile height>`
        let dims = first.trim().split_once('x').and_then(|(x, y)| Some((usize::try_from(int(x)?).ok()?, usize::try_from(int(y)?).ok()?)));
        (tile_x, tile_y) = dims.ok_or_else(|| format!("Invalid image metadata: {meta}"))?;
    }
    if tile_x == 0 || tile_y == 0 || img.width % tile_x != 0 || img.height % tile_y != 0 {
        return Err(format!("Invalid image metadata: {meta}"));
    }
    let (tx, ty) = (tile_x, tile_y);
    index.p2(tx as i64);
    index.p2(ty as i64);

    let colors = palette(&img);
    if colors.len() > 255 {
        return Err(format!("{src_path}/{safe_name}.png has {} colours: quantise it to at most 255", colors.len()));
    }
    index.p1(colors.len() as i64);
    for &c in &colors[1..] {
        index.p3(c as i64);
    }

    if tx != img.width || ty != img.height {
        for y in 0..img.height / ty {
            for x in 0..img.width / tx {
                write_image(&img.crop(x * tx, y * ty, tx, ty), &mut data, index, &colors);
            }
        }
    } else {
        write_image(&img, &mut data, index, &colors);
    }
    Ok(data)
}
