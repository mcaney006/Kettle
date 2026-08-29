//! Generate Kettle's app icon procedurally, with no dependencies.
//!
//! Port of the original Python/Pillow script. Everything is drawn at 4x and box
//! averaged down, so the supersampling supplies the antialiasing and the
//! rasteriser itself can stay a simple hard-edged scanline fill.
//!
//! Strokes are built as explicit offset-curve polygons rather than a wide-line
//! primitive, which produces visible notches at sharp joins and cannot taper.

use std::f64::consts::PI;

const S: usize = 1024; // final size
const SS: usize = 4; // supersample factor
const W: usize = S * SS;
const WF: f64 = W as f64;

type Pt = (f64, f64);

// ---------------------------------------------------------------- raster

/// RGBA8 image. Straight (non-premultiplied) alpha.
struct Img {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Img {
    fn new(w: usize, h: usize) -> Self {
        Img { w, h, px: vec![0; w * h * 4] }
    }

    fn blend(&mut self, x: usize, y: usize, c: [u8; 4]) {
        let i = (y * self.w + x) * 4;
        let a = c[3] as u32;
        if a == 0 {
            return;
        }
        if a == 255 {
            self.px[i..i + 4].copy_from_slice(&c);
            return;
        }
        // Source-over onto an opaque-ish plate.
        for k in 0..3 {
            let dst = self.px[i + k] as u32;
            self.px[i + k] = ((c[k] as u32 * a + dst * (255 - a)) / 255) as u8;
        }
        let da = self.px[i + 3] as u32;
        self.px[i + 3] = (a + da * (255 - a) / 255).min(255) as u8;
    }
}

/// Even-odd scanline fill. Adequate here because every polygon we draw is a
/// simple closed loop, and the 4x downsample provides the edge antialiasing.
fn fill_poly(img: &mut Img, pts: &[Pt], color: [u8; 4]) {
    if pts.len() < 3 {
        return;
    }
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in pts {
        lo = lo.min(p.1);
        hi = hi.max(p.1);
    }
    let y0 = lo.floor().max(0.0) as usize;
    let y1 = (hi.ceil() as isize).min(img.h as isize - 1);
    if y1 < 0 {
        return;
    }
    let mut xs: Vec<f64> = Vec::with_capacity(16);
    for y in y0..=(y1 as usize) {
        let sy = y as f64 + 0.5;
        xs.clear();
        for i in 0..pts.len() {
            let (x1p, y1p) = pts[i];
            let (x2p, y2p) = pts[(i + 1) % pts.len()];
            // Half-open rule stops double-counting shared vertices.
            if (y1p <= sy && y2p > sy) || (y2p <= sy && y1p > sy) {
                let t = (sy - y1p) / (y2p - y1p);
                xs.push(x1p + t * (x2p - x1p));
            }
        }
        if xs.is_empty() {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut i = 0;
        while i + 1 < xs.len() {
            let a = xs[i].max(0.0).round() as isize;
            let b = xs[i + 1].min(img.w as f64).round() as isize;
            for x in a.max(0)..b.min(img.w as isize) {
                img.blend(x as usize, y, color);
            }
            i += 2;
        }
    }
}

// ---------------------------------------------------------------- geometry

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let mut o = [0u8; 3];
    for i in 0..3 {
        o[i] = (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t).round() as u8;
    }
    o
}

/// Superellipse |x|^n + |y|^n <= 1, the shape of an Apple icon plate.
///
/// A rounded rect is only G1-continuous where its arc meets the straight edge;
/// the superellipse is G2, which is what makes it read as "correct".
fn squircle_alpha(size: usize, n: f64) -> Vec<u8> {
    let mut m = vec![0u8; size * size];
    let r = size as f64 / 2.0;
    for y in 0..size {
        let ay = (((y as f64 + 0.5 - r) / r).abs()).powf(n);
        if ay > 1.0 {
            continue;
        }
        // Solve for the x half-width at this row instead of scanning every pixel.
        let half = (1.0 - ay).powf(1.0 / n) * r;
        let x0 = (r - half).max(0.0) as usize;
        let x1 = ((r + half).ceil() as usize).min(size);
        for x in x0..x1 {
            m[y * size + x] = 255;
        }
    }
    m
}

/// Offset-curve polygon around a polyline, tapering from `w0` to `w1`.
///
/// For each sample we take the unit normal of the local tangent and push the
/// centreline out by half the interpolated width, walking one side forward and
/// the other back so the result is a single closed polygon.
fn ribbon(pts: &[Pt], w0: f64, w1: f64) -> Vec<Pt> {
    let n = pts.len();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for (i, &(x, y)) in pts.iter().enumerate() {
        let t = i as f64 / (n - 1) as f64;
        // Central difference for a smooth tangent; one-sided at the ends.
        let (px, py) = pts[i.saturating_sub(1)];
        let (nx, ny) = pts[(i + 1).min(n - 1)];
        let (dx, dy) = (nx - px, ny - py);
        let l = dx.hypot(dy).max(1e-9);
        let (ox, oy) = (-dy / l, dx / l);
        let hw = (w0 + (w1 - w0) * t) / 2.0;
        left.push((x + ox * hw, y + oy * hw));
        right.push((x - ox * hw, y - oy * hw));
    }
    right.reverse();
    left.extend(right);
    left
}

/// Round every vertex of a convex polygon with a true tangent arc.
///
/// Trims back along both incident edges by the tangent length and joins with a
/// circular arc, so the fillet meets each edge tangentially. Overdrawing discs
/// and rectangles instead leaves steps wherever an edge is not axis-aligned.
fn rounded_poly(verts: &[Pt], r: f64, steps: usize) -> Vec<Pt> {
    let n = verts.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (px, py) = verts[(i + n - 1) % n];
        let (cx, cy) = verts[i];
        let (nx, ny) = verts[(i + 1) % n];

        let v1 = (px - cx, py - cy);
        let v2 = (nx - cx, ny - cy);
        let l1 = v1.0.hypot(v1.1).max(1e-9);
        let l2 = v2.0.hypot(v2.1).max(1e-9);
        let u1 = (v1.0 / l1, v1.1 / l1);
        let u2 = (v2.0 / l2, v2.1 / l2);

        // Interior half-angle between the two edges.
        let cosang = (u1.0 * u2.0 + u1.1 * u2.1).clamp(-1.0, 1.0);
        let half = cosang.acos() / 2.0;
        if half <= 1e-6 || half.tan().abs() < 1e-6 {
            out.push((cx, cy));
            continue;
        }
        // Tangent length, clamped so neighbouring fillets cannot overlap.
        let tl = (r / half.tan()).min(l1 / 2.0).min(l2 / 2.0);
        let rr = tl * half.tan();

        let t1 = (cx + u1.0 * tl, cy + u1.1 * tl);
        let t2 = (cx + u2.0 * tl, cy + u2.1 * tl);
        // Arc centre lies along the angle bisector.
        let (bx, by) = (u1.0 + u2.0, u1.1 + u2.1);
        let bl = bx.hypot(by).max(1e-9);
        let dist = rr.hypot(tl);
        let (ox, oy) = (cx + bx / bl * dist, cy + by / bl * dist);

        let a1 = (t1.1 - oy).atan2(t1.0 - ox);
        let mut a2 = (t2.1 - oy).atan2(t2.0 - ox);
        // Take the short way round.
        while a2 - a1 > PI {
            a2 -= 2.0 * PI;
        }
        while a1 - a2 > PI {
            a2 += 2.0 * PI;
        }
        for s in 0..=steps {
            let a = a1 + (a2 - a1) * s as f64 / steps as f64;
            out.push((ox + rr * a.cos(), oy + rr * a.sin()));
        }
    }
    out
}

fn arc_pts(cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, steps: usize) -> Vec<Pt> {
    (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / steps as f64;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Three box blurs approximate a Gaussian closely enough for a soft sheen, and
/// run in O(n) per pass instead of O(n * radius).
fn box_blur(mask: &mut Vec<u8>, w: usize, h: usize, radius: usize) {
    if radius == 0 {
        return;
    }
    let mut tmp = vec![0u8; w * h];
    for _ in 0..3 {
        // horizontal
        for y in 0..h {
            let row = y * w;
            let mut sum: u32 = 0;
            let span = (radius * 2 + 1) as u32;
            for x in 0..radius.min(w) {
                sum += mask[row + x] as u32;
            }
            for x in 0..w {
                let add = (x + radius).min(w - 1);
                let sub = x as isize - radius as isize - 1;
                sum += mask[row + add] as u32;
                if sub >= 0 {
                    sum -= mask[row + sub as usize] as u32;
                }
                tmp[row + x] = (sum / span).min(255) as u8;
            }
        }
        // vertical
        for x in 0..w {
            let mut sum: u32 = 0;
            let span = (radius * 2 + 1) as u32;
            for y in 0..radius.min(h) {
                sum += tmp[y * w + x] as u32;
            }
            for y in 0..h {
                let add = (y + radius).min(h - 1);
                let sub = y as isize - radius as isize - 1;
                sum += tmp[add * w + x] as u32;
                if sub >= 0 {
                    sum -= tmp[sub as usize * w + x] as u32;
                }
                mask[y * w + x] = (sum / span).min(255) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------- png

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Minimal PNG writer. The zlib stream uses stored (uncompressed) deflate
/// blocks: the output is larger than a compressed one, but this is a build
/// intermediate that `sips` immediately turns into an .icns, and it saves
/// pulling in a compression dependency.
fn write_png(path: &str, w: usize, h: usize, rgba: &[u8]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity(h * (1 + w * 4));
    for y in 0..h {
        raw.push(0); // filter: none
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }

    let mut z = vec![0x78, 0x01]; // zlib header, no preset dict
    let mut i = 0;
    while i < raw.len() {
        let n = (raw.len() - i).min(65535);
        let last = if i + n >= raw.len() { 1u8 } else { 0u8 };
        z.push(last); // BTYPE=00, stored
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[i..i + n]);
        i += n;
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}

// ---------------------------------------------------------------- draw

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "icon.png".into());

    let top = [0xFF, 0xC1, 0x63];
    let bot = [0xC0, 0x41, 0x0E];
    let white = [255, 255, 255, 255];

    // --- plate: vertical gradient
    let mut img = Img::new(W, W);
    for y in 0..W {
        let c = lerp_rgb(top, bot, y as f64 / (W - 1) as f64);
        for x in 0..W {
            let i = (y * W + x) * 4;
            img.px[i] = c[0];
            img.px[i + 1] = c[1];
            img.px[i + 2] = c[2];
            img.px[i + 3] = 255;
        }
    }

    // --- sheen: a big soft ellipse of white across the top-left
    let mut sheen = vec![0u8; W * W];
    let (ex0, ey0, ex1, ey1) = (-0.35 * WF, -0.60 * WF, 0.85 * WF, 0.40 * WF);
    let (ecx, ecy) = ((ex0 + ex1) / 2.0, (ey0 + ey1) / 2.0);
    let (erx, ery) = ((ex1 - ex0) / 2.0, (ey1 - ey0) / 2.0);
    for y in 0..W {
        let dy = (y as f64 + 0.5 - ecy) / ery;
        if dy.abs() > 1.0 {
            continue;
        }
        let halfx = (1.0 - dy * dy).sqrt() * erx;
        let x0 = (ecx - halfx).max(0.0) as usize;
        let x1 = ((ecx + halfx).ceil().max(0.0) as usize).min(W);
        for x in x0..x1 {
            sheen[y * W + x] = 64;
        }
    }
    // Three box passes of half-width r approximate a Gaussian of sigma
    // sqrt(((2r+1)^2 - 1) / 4), i.e. r ~= sigma. Dividing by the pass count
    // instead leaves the highlight with a visible hard edge.
    box_blur(&mut sheen, W, W, (WF * 0.07) as usize);
    for y in 0..W {
        for x in 0..W {
            let s = sheen[y * W + x];
            if s > 0 {
                img.blend(x, y, [255, 255, 255, s]);
            }
        }
    }

    // --- clip to the icon plate
    let mask = squircle_alpha(W, 5.0);
    for i in 0..W * W {
        if mask[i] == 0 {
            img.px[i * 4 + 3] = 0;
        }
    }

    let cx = WF / 2.0;

    // --- steam: three tapered sine ribbons, fading as they rise
    for &(phase, xoff, amp, a) in &[
        (0.4f64, -0.135f64, 0.045f64, 200u8),
        (0.0, 0.0, 0.055, 245),
        (0.8, 0.135, 0.045, 200),
    ] {
        let pts: Vec<Pt> = (0..61)
            .map(|i| {
                let u = i as f64 / 60.0;
                let y = WF * (0.255 + 0.215 * u);
                let x = cx + WF * xoff + (u * PI * 1.5 + phase).sin() * WF * amp;
                (x, y)
            })
            .collect();
        // Wider at the bottom (near the spout), thinning as it dissipates.
        fill_poly(&mut img, &ribbon(&pts, WF * 0.020, WF * 0.040), [255, 255, 255, a]);
    }

    // --- body: rounded trapezoid, wider at the shoulder
    let (by, bh) = (WF * 0.545, WF * 0.245);
    let (tw, bw) = (WF * 0.44, WF * 0.375);
    let body = [
        (cx - tw / 2.0, by),
        (cx + tw / 2.0, by),
        (cx + bw / 2.0, by + bh),
        (cx - bw / 2.0, by + bh),
    ];
    fill_poly(&mut img, &rounded_poly(&body, WF * 0.05, 16), white);

    // --- spout: tapered ribbon sweeping up and right off the shoulder
    let sp = [
        (cx + tw * 0.40, by + bh * 0.30),
        (cx + tw * 0.62, by + bh * 0.22),
        (cx + tw * 0.78, by - bh * 0.02),
    ];
    let dense: Vec<Pt> = (0..41)
        .map(|i| {
            let u = i as f64 / 40.0;
            // Quadratic Bezier through the three control points.
            let m = 1.0 - u;
            (
                m * m * sp[0].0 + 2.0 * m * u * sp[1].0 + u * u * sp[2].0,
                m * m * sp[0].1 + 2.0 * m * u * sp[1].1 + u * u * sp[2].1,
            )
        })
        .collect();
    fill_poly(&mut img, &ribbon(&dense, WF * 0.075, WF * 0.038), white);

    // --- handle: semicircular band above the shoulder
    let h = arc_pts(cx, by + WF * 0.006, tw * 0.40, WF * 0.105, PI, 2.0 * PI, 80);
    fill_poly(&mut img, &ribbon(&h, WF * 0.030, WF * 0.030), white);

    // --- downsample: box average of each SS x SS block
    let mut small = vec![0u8; S * S * 4];
    let n = (SS * SS) as u32;
    for y in 0..S {
        for x in 0..S {
            let mut acc = [0u32; 4];
            for dy in 0..SS {
                for dx in 0..SS {
                    let i = ((y * SS + dy) * W + (x * SS + dx)) * 4;
                    for k in 0..4 {
                        acc[k] += img.px[i + k] as u32;
                    }
                }
            }
            let o = (y * S + x) * 4;
            for k in 0..4 {
                small[o + k] = (acc[k] / n) as u8;
            }
        }
    }

    write_png(&out, S, S, &small).expect("write png");
    println!("wrote {out}");
}
