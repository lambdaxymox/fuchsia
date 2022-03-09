// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::{fmt, sync::Arc};

use crate::{simd::f32x8, AffineTransform, Point};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Red,
    Green,
    Blue,
    Alpha,
}

impl Channel {
    pub(crate) fn select<T>(self, r: T, g: T, b: T, a: T) -> T {
        match self {
            Channel::Red => r,
            Channel::Green => g,
            Channel::Blue => b,
            Channel::Alpha => a,
        }
    }

    pub(crate) fn select_from_color(self, color: Color) -> f32 {
        match self {
            Channel::Red => color.r,
            Channel::Green => color.g,
            Channel::Blue => color.b,
            Channel::Alpha => color.a,
        }
    }
}

pub const RGBA: [Channel; 4] = [Channel::Red, Channel::Green, Channel::Blue, Channel::Alpha];
pub const BGRA: [Channel; 4] = [Channel::Blue, Channel::Green, Channel::Red, Channel::Alpha];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub(crate) fn max(&self) -> f32 {
        self.r.max(self.g.max(self.b))
    }
    pub(crate) fn min(&self) -> f32 {
        self.r.min(self.g.min(self.b))
    }

    pub(crate) fn sorted(&mut self) -> [&mut f32; 3] {
        let c = [self.r, self.g, self.b];

        match (c[0] < c[1], c[0] < c[2], c[1] < c[2]) {
            (true, true, true) => [&mut self.r, &mut self.g, &mut self.b],
            (true, true, false) => [&mut self.r, &mut self.b, &mut self.g],
            (true, false, _) => [&mut self.b, &mut self.r, &mut self.g],
            (false, true, true) => [&mut self.g, &mut self.r, &mut self.b],
            (false, _, false) => [&mut self.b, &mut self.g, &mut self.r],
            (false, false, true) => [&mut self.g, &mut self.b, &mut self.r],
        }
    }

    pub(crate) fn channel(&self, c: Channel) -> f32 {
        match c {
            Channel::Red => self.r,
            Channel::Green => self.g,
            Channel::Blue => self.b,
            Channel::Alpha => self.a,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

impl Default for FillRule {
    fn default() -> Self {
        Self::NonZero
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientType {
    Linear,
    Radial,
}

const NO_STOP: f32 = -0.0;

#[derive(Clone, Debug)]
pub struct GradientBuilder {
    r#type: GradientType,
    start: Point,
    end: Point,
    stops: Vec<(Color, f32)>,
}

impl GradientBuilder {
    pub fn new(start: Point, end: Point) -> Self {
        Self { r#type: GradientType::Linear, start, end, stops: Vec::new() }
    }

    pub fn r#type(&mut self, r#type: GradientType) -> &mut Self {
        self.r#type = r#type;
        self
    }

    pub fn color(&mut self, color: Color) -> &mut Self {
        self.stops.push((color, NO_STOP));
        self
    }

    pub fn color_with_stop(&mut self, color: Color, stop: f32) -> &mut Self {
        if !(0.0..=1.0).contains(&stop) {
            panic!("gradient stops must be between 0.0 and 1.0");
        }

        self.stops.push((color, stop));
        self
    }

    pub fn build(mut self) -> Option<Gradient> {
        if self.stops.len() < 2 {
            return None;
        }

        let stop_increment = 1.0 / (self.stops.len() - 1) as f32;
        for (i, (_, stop)) in self.stops.iter_mut().enumerate() {
            if *stop == NO_STOP {
                *stop = i as f32 * stop_increment;
            }
        }

        Some(Gradient {
            r#type: self.r#type,
            start: self.start,
            end: self.end,
            stops: self.stops.into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    r#type: GradientType,
    start: Point,
    end: Point,
    stops: Arc<[(Color, f32)]>,
}

impl Gradient {
    fn get_t(&self, x: f32, y: f32) -> f32x8 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;

        let dot = dx * dx + dy * dy;
        let dot_recip = dot.recip();

        match self.r#type {
            GradientType::Linear => {
                let tx = (x - self.start.x) * dx * dot_recip;
                let ty = y - self.start.y;

                ((f32x8::indexed() + f32x8::splat(ty)) * f32x8::splat(dy))
                    .mul_add(f32x8::splat(dot_recip), f32x8::splat(tx))
            }
            GradientType::Radial => {
                let px = x - self.start.x;
                let px2 = f32x8::splat(px * px);
                let py = f32x8::indexed() + f32x8::splat(y - self.start.y);

                (py.mul_add(py, px2) * f32x8::splat(dot_recip)).sqrt()
            }
        }
    }

    pub(crate) fn color_at(&self, x: f32, y: f32) -> [f32x8; 4] {
        let mut channels = [f32x8::splat(0.0); 4];

        let t = self.get_t(x, y);

        let mask = t.le(f32x8::splat(self.stops[0].1));
        if mask.any() {
            let stop = self.stops[0].0;
            for (channel, &stop_channel) in
                channels.iter_mut().zip([stop.r, stop.g, stop.b, stop.a].iter())
            {
                *channel |= f32x8::splat(stop_channel).select(f32x8::splat(0.0), mask);
            }
        }

        let mut start_stop = 0.0;
        let mut start_color = self.stops[0].0;
        let mut acc_mask = mask;

        for &(color, end_stop) in self.stops.iter().skip(1) {
            let mask = acc_mask ^ t.lt(f32x8::splat(end_stop));
            if mask.any() {
                let d = end_stop - start_stop;
                let local_t = (t - f32x8::splat(start_stop)) * f32x8::splat(d.recip());

                for (channel, (&start_channel, &end_channel)) in channels.iter_mut().zip(
                    [start_color.r, start_color.g, start_color.b, start_color.a]
                        .iter()
                        .zip([color.r, color.g, color.b, color.a].iter()),
                ) {
                    *channel |= local_t
                        .mul_add(
                            f32x8::splat(end_channel),
                            (-local_t)
                                .mul_add(f32x8::splat(start_channel), f32x8::splat(start_channel)),
                        )
                        .select(f32x8::splat(0.0), mask);
                }

                acc_mask |= mask;
            }

            start_stop = end_stop;
            start_color = color;
        }

        let mask = !acc_mask;
        if mask.any() {
            let stop = self.stops[self.stops.len() - 1].0;
            for (channel, &stop_channel) in
                channels.iter_mut().zip([stop.r, stop.g, stop.b, stop.a].iter())
            {
                *channel |= f32x8::splat(stop_channel).select(f32x8::splat(0.0), mask);
            }
        }

        channels
    }
}
#[derive(Debug)]
pub enum ImageError {
    SizeMismatch { len: usize, width: usize, height: usize },
    TooLarge,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeMismatch { len, width, height } => {
                write!(
                    f,
                    "buffer has {} pixels, which does not match \
                     the specified width ({}) and height ({})",
                    len, width, height
                )
            }
            Self::TooLarge => {
                write!(
                    f,
                    "image dimensions exceed what is addressable \
                     with f32; try to reduce the image size."
                )
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Image pixels stored as f32 components xyza in linear space.
    /// The array is expected to contain width * height elements.
    data: Box<[[f32; 4]]>,
    /// Width of the image.
    width: f32,
    /// Height of the image.
    height: f32,
}

impl Image {
    pub fn new(data: Box<[[f32; 4]]>, width: usize, height: usize) -> Result<Image, ImageError> {
        match width * height {
            len if len > (1 << f32::MANTISSA_DIGITS) => Err(ImageError::TooLarge),
            len if len != data.len() => {
                Err(ImageError::SizeMismatch { len: data.len(), width, height })
            }
            _ => Ok(Image { data, width: width as f32, height: height as f32 }),
        }
    }
}

/// Describes how to shade a surface using a bitmap image.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    /// Transformation from screen-space to texture-space.
    pub transform: AffineTransform,
    /// Image shared with zero or more textures.
    pub image: Arc<Image>,
}

impl Texture {
    #[inline(never)]
    pub fn color_at(&self, x: f32, y: f32) -> [f32x8; 4] {
        let x = f32x8::splat(x);
        let y = f32x8::splat(y) + f32x8::indexed();
        // Apply affine transformation.
        let t = self.transform;
        let tx = x.mul_add(f32x8::splat(t.ux), f32x8::splat(t.vx).mul_add(y, f32x8::splat(t.tx)));
        let ty = x.mul_add(f32x8::splat(t.uy), f32x8::splat(t.vy).mul_add(y, f32x8::splat(t.ty)));
        // Apply Clamp texture mode.
        let tx = tx.clamp(f32x8::splat(0.0), f32x8::splat(self.image.width - 1.0)).floor();
        let ty = ty.clamp(f32x8::splat(0.0), f32x8::splat(self.image.height - 1.0)).floor();
        // Compute texture offsets.
        // Largest consecutive integer is 2^53
        let offsets = ty.mul_add(f32x8::splat(self.image.width as f32), tx);
        let data = &*self.image.data;
        // TODO(fxb/94997): Evaluate SIMD conversion to u32x8.
        let pixels = offsets.to_array().map(|o| data[o as usize]);
        let get_channel = |c| {
            f32x8::from_array([
                pixels[0][c],
                pixels[1][c],
                pixels[2][c],
                pixels[3][c],
                pixels[4][c],
                pixels[5][c],
                pixels[6][c],
                pixels[7][c],
            ])
        };
        [get_channel(0), get_channel(1), get_channel(2), get_channel(3)]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    Gradient(Gradient),
    Texture(Texture),
}

impl Default for Fill {
    fn default() -> Self {
        Self::Solid(Color::default())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlendMode {
    Over,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub(crate) fn blend_fn(self) -> fn(Channel, Color, Color) -> f32 {
        fn multiply(dst: f32, src: f32) -> f32 {
            dst * src
        }

        fn screen(dst: f32, src: f32) -> f32 {
            dst + src - (dst * src)
        }

        fn hard_light(dst: f32, src: f32) -> f32 {
            if dst <= 0.5 {
                multiply(2.0 * dst, src)
            } else {
                screen(2.0 * dst - 1.0, src)
            }
        }

        fn lum(color: Color) -> f32 {
            color.r.mul_add(0.3, color.g.mul_add(0.59, color.b * 0.11))
        }

        fn clip_color(c: Channel, color: Color) -> f32 {
            let l = lum(color);
            let n = color.min();
            let x = color.max();
            let mut c = color.channel(c);

            if n < 0.0 {
                let l_n_recip_l = (l - n).recip() * l;
                c = l_n_recip_l.mul_add(c - l, l);
            }

            if x > 1.0 {
                let l_1 = l - 1.0;
                let x_l_recip = (x - l).recip();
                c = x_l_recip.mul_add(l.mul_add(l_1 - c, c), l);
            }
            c
        }

        fn set_lum(c: Channel, mut color: Color, l: f32) -> f32 {
            let d = l - lum(color);
            color.r += d;
            color.g += d;
            color.b += d;
            clip_color(c, color)
        }

        fn sat(color: Color) -> f32 {
            color.max() - color.min()
        }

        fn set_sat(mut color: Color, s: f32) -> Color {
            let [c_min, c_mid, c_max] = color.sorted();
            if c_max > c_min {
                *c_mid = s.mul_add(*c_mid, -s * *c_min) / (*c_max - *c_min);
                *c_max = s;
            } else {
                *c_mid = 0.0;
                *c_max = 0.0;
            }
            *c_min = 0.0;
            color
        }

        match self {
            Self::Over => |c, _, src| src.channel(c),
            Self::Multiply => |c, dst, src| multiply(dst.channel(c), src.channel(c)),
            Self::Screen => |c, dst, src| screen(dst.channel(c), src.channel(c)),
            Self::Overlay => |c, dst, src| hard_light(src.channel(c), dst.channel(c)),
            Self::Darken => |c, dst, src| dst.channel(c).min(src.channel(c)),
            Self::Lighten => |c, dst, src| dst.channel(c).max(src.channel(c)),
            Self::ColorDodge => |c, dst, src| {
                if src.channel(c) == 0.0 {
                    0.0
                } else if dst.channel(c) == 1.0 {
                    1.0
                } else {
                    1.0f32.min(src.channel(c) / (1.0 - dst.channel(c)))
                }
            },
            Self::ColorBurn => |c, dst, src| {
                if src.channel(c) == 1.0 {
                    1.0
                } else if dst.channel(c) == 0.0 {
                    0.0
                } else {
                    1.0 - 1.0f32.min((1.0 - src.channel(c)) / dst.channel(c))
                }
            },
            Self::HardLight => |c, dst, src| hard_light(dst.channel(c), src.channel(c)),
            Self::SoftLight => |c, dst, src| {
                fn d(src: f32) -> f32 {
                    if src <= 0.25 {
                        ((16.0 * src - 12.0) * src + 4.0) * src
                    } else {
                        src.sqrt()
                    }
                }

                if dst.channel(c) <= 0.5 {
                    src.channel(c)
                        - (1.0 - 2.0 * dst.channel(c)) * src.channel(c) * (1.0 - src.channel(c))
                } else {
                    src.channel(c)
                        + (2.0 * dst.channel(c) - 1.0) * (d(src.channel(c)) - src.channel(c))
                }
            },
            Self::Difference => |c, dst, src| (dst.channel(c) - src.channel(c)).abs(),
            Self::Exclusion => |c, dst, src| {
                dst.channel(c) + src.channel(c) - 2.0 * dst.channel(c) * src.channel(c)
            },
            Self::Color => |c, dst, src| set_lum(c, src, lum(dst)),
            Self::Luminosity => |c, dst, src| set_lum(c, dst, lum(src)),
            Self::Hue => |c, dst, src| set_lum(c, set_sat(src, sat(dst)), lum(dst)),
            Self::Saturation => |c, dst, src| set_lum(c, set_sat(dst, sat(src)), lum(dst)),
        }
    }

    pub(crate) fn blend(self, dst: Color, src: Color) -> Color {
        let f = self.blend_fn();

        let alpha = src.a;
        let inv_alpha = 1.0 - alpha;

        let current_red = f(Channel::Red, dst, src) * alpha;
        let current_green = f(Channel::Green, dst, src) * alpha;
        let current_blue = f(Channel::Blue, dst, src) * alpha;

        Color {
            r: dst.r.mul_add(inv_alpha, current_red),
            g: dst.g.mul_add(inv_alpha, current_green),
            b: dst.b.mul_add(inv_alpha, current_blue),
            a: dst.a.mul_add(inv_alpha, alpha),
        }
    }
}

macro_rules! blend_function {
    (
        $mode:expr,
        $dst_r:expr,
        $dst_g:expr,
        $dst_b:expr,
        $src_r:expr,
        $src_g:expr,
        $src_b:expr
        $( , )?
    ) => {{
        macro_rules! lum {
            ($r:expr, $g:expr, $b:expr) => {
                $r.mul_add(
                    f32x8::splat(0.3),
                    $g.mul_add(f32x8::splat(0.59), $b * f32x8::splat(0.11)),
                )
            };
        }

        macro_rules! sat {
            ($r:expr, $g:expr, $b:expr) => {
                $r.max($g.max($b)) - $r.min($g.min($b))
            };
        }

        macro_rules! clip_color {
            ($r:expr, $g:expr, $b:expr) => {{
                let l = lum!($r, $g, $b);
                let n = $r.min($g.min($b));
                let x = $r.max($g.max($b));
                let l_1 = l - f32x8::splat(1.0);
                let x_l_recip = (x - l).recip();
                let l_n_recip_l = (l - n).recip() * l;

                [
                    x_l_recip.mul_add(l.mul_add(l_1 - $r, $r), l).select(
                        l_n_recip_l.mul_add($r - l, l).select($r, n.lt(f32x8::splat(0.0))),
                        f32x8::splat(1.0).lt(x),
                    ),
                    x_l_recip.mul_add(l.mul_add(l_1 - $g, $g), l).select(
                        l_n_recip_l.mul_add($g - l, l).select($g, n.lt(f32x8::splat(0.0))),
                        f32x8::splat(1.0).lt(x),
                    ),
                    x_l_recip.mul_add(l.mul_add(l_1 - $b, $b), l).select(
                        l_n_recip_l.mul_add($b - l, l).select($b, n.lt(f32x8::splat(0.0))),
                        f32x8::splat(1.0).lt(x),
                    ),
                ]
            }};
        }

        macro_rules! set_lum {
            ($r:expr, $g:expr, $b:expr, $l:expr) => {{
                let d = $l - lum!($r, $g, $b);
                $r += d;
                $g += d;
                $b += d;
                clip_color!($r, $g, $b)
            }};
        }

        macro_rules! set_sat {
            ($sat_dst:expr, $s_r:expr, $s_g:expr, $s_b:expr) => {{
                let src_min = $s_r.min($s_g.min($s_b));
                let src_max = $s_r.max($s_g.max($s_b));
                let src_mid = $s_r + $s_g + $s_b - src_min - src_max;
                let min_lt_max = src_min.lt(src_max);
                let sat_mid = ($sat_dst.mul_add(-src_min, $sat_dst * src_mid)
                    / (src_max - src_min))
                    .select(f32x8::splat(0.0), min_lt_max);
                let sat_max = $sat_dst.select(f32x8::splat(0.0), min_lt_max);

                [
                    sat_mid.select(
                        f32x8::splat(0.0).select(sat_max, $s_r.eq(src_min)),
                        $s_r.eq(src_mid),
                    ),
                    sat_mid.select(
                        f32x8::splat(0.0).select(sat_max, $s_g.eq(src_min)),
                        $s_g.eq(src_mid),
                    ),
                    sat_mid.select(
                        f32x8::splat(0.0).select(sat_max, $s_b.eq(src_min)),
                        $s_b.eq(src_mid),
                    ),
                ]
            }};
        }

        match $mode {
            BlendMode::Over => [$src_r, $src_g, $src_b],
            BlendMode::Multiply => [$dst_r * $src_r, $dst_g * $src_g, $dst_b * $src_b],
            BlendMode::Screen => [
                $dst_r.mul_add(-$src_r, $dst_r) + $src_r,
                $dst_g.mul_add(-$src_g, $dst_g) + $src_g,
                $dst_b.mul_add(-$src_b, $dst_b) + $src_b,
            ],
            BlendMode::Overlay => [
                ($dst_r * $src_r * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_r + $dst_r - $src_r.mul_add($dst_r, f32x8::splat(0.5))),
                    $src_r.le(f32x8::splat(0.5)),
                ),
                ($dst_g * $src_g * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_g + $dst_g - $src_g.mul_add($dst_g, f32x8::splat(0.5))),
                    $src_g.le(f32x8::splat(0.5)),
                ),
                ($dst_b * $src_b * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_b + $dst_b - $src_b.mul_add($dst_b, f32x8::splat(0.5))),
                    $src_b.le(f32x8::splat(0.5)),
                ),
            ],
            BlendMode::Darken => [$dst_r.min($src_r), $dst_g.min($src_g), $dst_b.min($src_b)],
            BlendMode::Lighten => [$dst_r.max($src_r), $dst_g.max($src_g), $dst_b.max($src_b)],
            BlendMode::ColorDodge => [
                f32x8::splat(0.0).select(
                    f32x8::splat(1.0).min($src_r / (f32x8::splat(1.0) - $dst_r)),
                    $src_r.eq(f32x8::splat(0.0)),
                ),
                f32x8::splat(0.0).select(
                    f32x8::splat(1.0).min($src_g / (f32x8::splat(1.0) - $dst_g)),
                    $src_g.eq(f32x8::splat(0.0)),
                ),
                f32x8::splat(0.0).select(
                    f32x8::splat(1.0).min($src_b / (f32x8::splat(1.0) - $dst_b)),
                    $src_b.eq(f32x8::splat(0.0)),
                ),
            ],
            BlendMode::ColorBurn => [
                f32x8::splat(1.0).select(
                    f32x8::splat(1.0)
                        - f32x8::splat(1.0).min((f32x8::splat(1.0) - $src_r) / $dst_r),
                    $src_r.eq(f32x8::splat(1.0)),
                ),
                f32x8::splat(1.0).select(
                    f32x8::splat(1.0)
                        - f32x8::splat(1.0).min((f32x8::splat(1.0) - $src_g) / $dst_g),
                    $src_g.eq(f32x8::splat(1.0)),
                ),
                f32x8::splat(1.0).select(
                    f32x8::splat(1.0)
                        - f32x8::splat(1.0).min((f32x8::splat(1.0) - $src_b) / $dst_b),
                    $src_b.eq(f32x8::splat(1.0)),
                ),
            ],
            BlendMode::HardLight => [
                ($dst_r * $src_r * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_r + $dst_r - $src_r.mul_add($dst_r, f32x8::splat(0.5))),
                    $dst_r.le(f32x8::splat(0.5)),
                ),
                ($dst_g * $src_g * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_g + $dst_g - $src_g.mul_add($dst_g, f32x8::splat(0.5))),
                    $dst_g.le(f32x8::splat(0.5)),
                ),
                ($dst_b * $src_b * f32x8::splat(2.0)).select(
                    f32x8::splat(2.0)
                        * ($src_b + $dst_b - $src_b.mul_add($dst_b, f32x8::splat(0.5))),
                    $dst_b.le(f32x8::splat(0.5)),
                ),
            ],
            BlendMode::SoftLight => {
                let d0 = (f32x8::splat(16.0)
                    .mul_add($src_r, f32x8::splat(-12.0))
                    .mul_add($src_r, f32x8::splat(4.0))
                    * $src_r)
                    .select($src_r.sqrt(), $src_r.le(f32x8::splat(0.25)));
                let d1 = (f32x8::splat(16.0)
                    .mul_add($src_g, f32x8::splat(-12.0))
                    .mul_add($src_g, f32x8::splat(4.0))
                    * $src_g)
                    .select($src_g.sqrt(), $src_g.le(f32x8::splat(0.25)));
                let d2 = (f32x8::splat(16.0)
                    .mul_add($src_b, f32x8::splat(-12.0))
                    .mul_add($src_b, f32x8::splat(4.0))
                    * $src_b)
                    .select($src_b.sqrt(), $src_b.le(f32x8::splat(0.25)));

                [
                    (($src_r * (f32x8::splat(1.0) - $src_r))
                        .mul_add(f32x8::splat(2.0).mul_add($dst_r, f32x8::splat(-1.0)), $src_r))
                    .select(
                        (d0 - $src_r)
                            .mul_add(f32x8::splat(2.0).mul_add($dst_r, f32x8::splat(-1.0)), $src_r),
                        $dst_r.le(f32x8::splat(0.5)),
                    ),
                    (($src_g * (f32x8::splat(1.0) - $src_g))
                        .mul_add(f32x8::splat(2.0).mul_add($dst_g, f32x8::splat(-1.0)), $src_g))
                    .select(
                        (d1 - $src_g)
                            .mul_add(f32x8::splat(2.0).mul_add($dst_g, f32x8::splat(-1.0)), $src_g),
                        $dst_g.le(f32x8::splat(0.5)),
                    ),
                    (($src_b * (f32x8::splat(1.0) - $src_b))
                        .mul_add(f32x8::splat(2.0).mul_add($dst_b, f32x8::splat(-1.0)), $src_b))
                    .select(
                        (d2 - $src_b)
                            .mul_add(f32x8::splat(2.0).mul_add($dst_b, f32x8::splat(-1.0)), $src_b),
                        $dst_b.le(f32x8::splat(0.5)),
                    ),
                ]
            }
            BlendMode::Difference => {
                [($dst_r - $src_r).abs(), ($dst_g - $src_g).abs(), ($dst_b - $src_b).abs()]
            }
            BlendMode::Exclusion => [
                (f32x8::splat(-2.0) * $dst_r).mul_add($src_r, $dst_r) + $src_r,
                (f32x8::splat(-2.0) * $dst_g).mul_add($src_g, $dst_g) + $src_g,
                (f32x8::splat(-2.0) * $dst_b).mul_add($src_b, $dst_b) + $src_b,
            ],
            BlendMode::Hue => {
                let mut src = set_sat!(sat!($dst_r, $dst_g, $dst_b), $src_r, $src_g, $src_b);
                set_lum!(src[0], src[1], src[2], lum!($dst_r, $dst_g, $dst_b))
            }
            BlendMode::Saturation => {
                let mut dst = set_sat!(sat!($src_r, $src_g, $src_b), $dst_r, $dst_g, $dst_b);
                set_lum!(dst[0], dst[1], dst[2], lum!($dst_r, $dst_g, $dst_b))
            }
            BlendMode::Color => {
                let mut src = [$src_r, $src_g, $src_b];
                set_lum!(src[0], src[1], src[2], lum!($dst_r, $dst_g, $dst_b))
            }
            BlendMode::Luminosity => {
                let mut dst = [$dst_r, $dst_g, $dst_b];
                set_lum!(dst[0], dst[1], dst[2], lum!($src_r, $src_g, $src_b))
            }
        }
    }};
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::Over
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub is_clipped: bool,
    pub fill: Fill,
    pub blend_mode: BlendMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.001;

    macro_rules! color {
        ( $val:expr ) => {
            Color { r: $val, g: $val, b: $val, a: $val }
        };
    }

    macro_rules! color_array {
        ( $val:expr ) => {
            [$val, $val, $val, $val]
        };
    }

    macro_rules! color_eq {
        ( $val:expr ) => {{
            assert_eq!($val[0], $val[1]);
            assert_eq!($val[1], $val[2]);
            assert_eq!($val[2], $val[3]);

            $val[0]
        }};
    }

    macro_rules! simd_assert_approx {
        ( $left:expr, $right:expr ) => {{
            assert!(
                ($left - $right).abs().le(f32x8::splat(EPSILON)).all(),
                "{:?} != {:?}",
                $left,
                $right,
            );
        }};
    }

    fn colors(separate: &[f32x8; 4]) -> [[f32; 4]; 8] {
        let mut colors = [[0.0, 0.0, 0.0, 0.0]; 8];

        for (i, color) in colors.iter_mut().enumerate() {
            *color = [
                separate[0].to_array()[i],
                separate[1].to_array()[i],
                separate[2].to_array()[i],
                separate[3].to_array()[i],
            ];
        }

        colors
    }

    fn test_blend_mode(blend_mode: BlendMode) {
        let color_values = [
            Color { r: 0.125, g: 0.25, b: 0.625, a: 0.5 },
            Color { r: 0.25, g: 0.125, b: 0.75, a: 0.5 },
            Color { r: 0.625, g: 0.5, b: 0.125, a: 0.5 },
            Color { r: 0.375, g: 1.0, b: 0.875, a: 0.5 },
            Color { r: 0.5, g: 0.5, b: 0.5, a: 0.5 },
            Color { r: 0.875, g: 0.125, b: 0.0, a: 0.5 },
        ];
        let f = blend_mode.blend_fn();

        for &dst in &color_values {
            for &src in &color_values {
                let [r, g, b] = blend_function!(
                    blend_mode,
                    f32x8::splat(dst.r),
                    f32x8::splat(dst.g),
                    f32x8::splat(dst.b),
                    f32x8::splat(src.r),
                    f32x8::splat(src.g),
                    f32x8::splat(src.b),
                );

                simd_assert_approx!(r, f32x8::splat(f(Channel::Red, dst, src)));
                simd_assert_approx!(g, f32x8::splat(f(Channel::Green, dst, src)));
                simd_assert_approx!(b, f32x8::splat(f(Channel::Blue, dst, src)));
            }
        }
    }

    #[test]
    fn linear_gradient() {
        let mut builder = GradientBuilder::new(Point::new(0.0, 7.0), Point::new(7.0, 0.0));

        builder
            .color(color!(0.25))
            .color(color!(0.75))
            .color(color!(0.25))
            .color(color!(0.75))
            .color(color!(0.25));

        let gradient = builder.build().unwrap();

        let col = colors(&gradient.color_at(0.0, 0.0));
        assert_eq!(col[0], color_array!(0.25));
        assert!(color_eq!(col[1]) < color_eq!(col[2]));
        assert!(color_eq!(col[2]) < color_eq!(col[3]));
        assert!(color_eq!(col[4]) > color_eq!(col[5]));
        assert!(color_eq!(col[5]) > color_eq!(col[6]));
        assert_eq!(col[7], color_array!(0.25));

        let col = colors(&gradient.color_at(3.0, 0.0));
        assert!(color_eq!(col[0]) < 0.75);
        assert!(color_eq!(col[1]) > color_eq!(col[2]));
        assert!(color_eq!(col[2]) > color_eq!(col[3]));
        assert_eq!(col[3], color_array!(0.25));
        assert!(color_eq!(col[3]) < color_eq!(col[4]));
        assert!(color_eq!(col[4]) < color_eq!(col[5]));
        assert!(color_eq!(col[5]) < color_eq!(col[6]));
        assert!(color_eq!(col[7]) < 0.75);

        let col = colors(&gradient.color_at(7.0, 0.0));
        assert_eq!(col[0], color_array!(0.25));
        assert!(color_eq!(col[1]) < color_eq!(col[2]));
        assert!(color_eq!(col[2]) < color_eq!(col[3]));
        assert!(color_eq!(col[4]) > color_eq!(col[5]));
        assert!(color_eq!(col[5]) > color_eq!(col[6]));
        assert_eq!(col[7], color_array!(0.25));
    }

    #[test]
    fn radial_gradient() {
        let mut builder = GradientBuilder::new(
            Point::new(0.0, 0.0),
            Point::new(7.0 * (1.0 / 2.0f32.sqrt()), 7.0 * (1.0 / 2.0f32.sqrt())),
        );

        builder.r#type(GradientType::Radial).color(color!(0.25)).color(color!(0.75));

        let gradient = builder.build().unwrap();

        let col = colors(&gradient.color_at(0.0, 0.0));
        assert_eq!(col[0], color_array!(0.25));
        assert!(color_eq!(col[1]) < color_eq!(col[2]));
        assert!(color_eq!(col[2]) < color_eq!(col[3]));
        assert!(color_eq!(col[3]) < color_eq!(col[4]));
        assert!(color_eq!(col[4]) < color_eq!(col[5]));
        assert!(color_eq!(col[5]) < color_eq!(col[6]));
        assert_eq!(col[7], color_array!(0.75));

        let col = colors(&gradient.color_at(3.0, 0.0));
        assert!(color_eq!(col[0]) < color_eq!(col[1]));
        assert!(color_eq!(col[1]) < color_eq!(col[2]));
        assert!(color_eq!(col[2]) < color_eq!(col[3]));
        assert!(color_eq!(col[3]) < color_eq!(col[4]));
        assert!(color_eq!(col[4]) < color_eq!(col[5]));
        assert!(color_eq!(col[5]) < color_eq!(col[6]));
        assert_eq!(col[7], color_array!(0.75));

        let col = colors(&gradient.color_at(4.0, 0.0));
        assert!(color_eq!(col[0]) < color_eq!(col[1]));
        assert!(color_eq!(col[1]) < color_eq!(col[2]));
        assert!(color_eq!(col[2]) < color_eq!(col[3]));
        assert!(color_eq!(col[3]) < color_eq!(col[4]));
        assert!(color_eq!(col[4]) < color_eq!(col[5]));
        assert_eq!(col[6], color_array!(0.75));
        assert_eq!(col[7], color_array!(0.75));

        let col = colors(&gradient.color_at(7.0, 0.0));
        assert_eq!(col[0], color_array!(0.75));
        assert_eq!(col[1], color_array!(0.75));
        assert_eq!(col[2], color_array!(0.75));
        assert_eq!(col[3], color_array!(0.75));
        assert_eq!(col[4], color_array!(0.75));
        assert_eq!(col[5], color_array!(0.75));
        assert_eq!(col[6], color_array!(0.75));
        assert_eq!(col[7], color_array!(0.75));
    }

    #[test]
    fn test_blend_mode_over() {
        test_blend_mode(BlendMode::Over);
    }

    #[test]
    fn test_blend_mode_multiply() {
        test_blend_mode(BlendMode::Multiply);
    }

    #[test]
    fn test_blend_mode_screen() {
        test_blend_mode(BlendMode::Screen);
    }

    #[test]
    fn test_blend_mode_overlay() {
        test_blend_mode(BlendMode::Overlay);
    }

    #[test]
    fn test_blend_mode_darken() {
        test_blend_mode(BlendMode::Darken);
    }

    #[test]
    fn test_blend_mode_lighten() {
        test_blend_mode(BlendMode::Lighten);
    }

    #[test]
    fn test_blend_mode_color_dodge() {
        test_blend_mode(BlendMode::ColorDodge);
    }

    #[test]
    fn test_blend_mode_color_burn() {
        test_blend_mode(BlendMode::ColorBurn);
    }

    #[test]
    fn test_blend_mode_hard_light() {
        test_blend_mode(BlendMode::HardLight);
    }

    #[test]
    fn test_blend_mode_soft_light() {
        test_blend_mode(BlendMode::SoftLight);
    }

    #[test]
    fn test_blend_mode_difference() {
        test_blend_mode(BlendMode::Difference);
    }

    #[test]
    fn test_blend_mode_exclusion() {
        test_blend_mode(BlendMode::Exclusion);
    }

    #[test]
    fn test_blend_mode_hue() {
        test_blend_mode(BlendMode::Hue);
    }

    #[test]
    fn test_blend_mode_saturation() {
        test_blend_mode(BlendMode::Saturation);
    }

    #[test]
    fn test_blend_mode_color() {
        test_blend_mode(BlendMode::Color);
    }

    #[test]
    fn test_blend_mode_luminosity() {
        test_blend_mode(BlendMode::Luminosity);
    }

    #[test]
    fn channel_select() {
        let channels: [Channel; 4] = [Channel::Blue, Channel::Green, Channel::Red, Channel::Alpha];
        let red = [3f32; 8];
        let green = [2f32; 8];
        let blue = [1f32; 8];
        let alpha = [1f32; 8];
        let color = channels.map(|c| c.select(red, green, blue, alpha));
        assert_eq!(color, [blue, green, red, alpha]);
    }

    #[test]
    fn channel_select_from_color() {
        let channels: [Channel; 4] = [Channel::Blue, Channel::Green, Channel::Red, Channel::Alpha];
        let color = Color { r: 3.0, g: 2.0, b: 1.0, a: 1.0 };
        let color = channels.map(|c| c.select_from_color(color));
        assert_eq!(color, [1.0, 2.0, 3.0, 1.0]);
    }

    #[test]
    fn color_sorted() {
        let permutations = [
            (1.0, 2.0, 3.0),
            (1.0, 3.0, 2.0),
            (2.0, 1.0, 3.0),
            (2.0, 3.0, 1.0),
            (3.0, 1.0, 2.0),
            (3.0, 2.0, 1.0),
        ];

        for (r, g, b) in permutations {
            let mut color = Color { r, g, b, a: 1.0 };
            let sorted = color.sorted();
            assert_eq!(sorted.map(|c| *c), [1.0, 2.0, 3.0]);
        }
    }

    #[test]
    fn color_min() {
        let color = Color { r: 3.0, g: 2.0, b: 1.0, a: 1.0 };
        let min = color.min();
        assert_eq!(min, 1.0);
    }

    #[test]
    fn color_max() {
        let color = Color { r: 3.0, g: 2.0, b: 1.0, a: 1.0 };
        let max = color.max();
        assert_eq!(max, 3.0);
    }

    fn apply_texture_color_at(transform: AffineTransform) -> Vec<[f32; 8]> {
        let image = Arc::new(
            Image::new(
                Box::new([
                    [0.1, 0.2, 0.3, 0.1],
                    [0.4, 0.5, 0.6, 0.2],
                    [0.7, 0.8, 0.9, 0.3],
                    [0.8, 0.7, 0.6, 0.4],
                    [0.5, 0.4, 0.5, 0.5],
                    [0.2, 0.1, 0.0, 0.6],
                ]),
                2,
                3,
            )
            .unwrap(),
        );
        let texture = Texture { transform, image };
        texture.color_at(-2.0, -2.0).iter().map(|v| v.to_array().clone()).collect()
    }

    #[test]
    fn texture_color_at_with_identity() {
        assert_eq!(
            apply_texture_color_at(AffineTransform::default()),
            [
                [0.1, 0.1, 0.1, 0.7, 0.5, 0.5, 0.5, 0.5],
                [0.2, 0.2, 0.2, 0.8, 0.4, 0.4, 0.4, 0.4],
                [0.3, 0.3, 0.3, 0.9, 0.5, 0.5, 0.5, 0.5],
                [0.1, 0.1, 0.1, 0.3, 0.5, 0.5, 0.5, 0.5],
            ],
        );
    }

    #[test]
    fn texture_color_at_with_scale_x2() {
        assert_eq!(
            apply_texture_color_at(AffineTransform {
                ux: 0.5,
                uy: 0.0,
                vy: 0.5,
                vx: 0.0,
                tx: 0.0,
                ty: 0.0
            }),
            [
                [0.1, 0.1, 0.1, 0.1, 0.7, 0.7, 0.5, 0.5],
                [0.2, 0.2, 0.2, 0.2, 0.8, 0.8, 0.4, 0.4],
                [0.3, 0.3, 0.3, 0.3, 0.9, 0.9, 0.5, 0.5],
                [0.1, 0.1, 0.1, 0.1, 0.3, 0.3, 0.5, 0.5],
            ],
        );
    }

    #[test]
    fn texture_color_at_with_translation() {
        assert_eq!(
            apply_texture_color_at(AffineTransform {
                ux: 1.0,
                uy: 0.0,
                vx: 0.0,
                vy: 1.0,
                tx: 1.0,
                ty: 1.0
            }),
            [
                [0.1, 0.1, 0.7, 0.5, 0.5, 0.5, 0.5, 0.5],
                [0.2, 0.2, 0.8, 0.4, 0.4, 0.4, 0.4, 0.4],
                [0.3, 0.3, 0.9, 0.5, 0.5, 0.5, 0.5, 0.5],
                [0.1, 0.1, 0.3, 0.5, 0.5, 0.5, 0.5, 0.5],
            ],
        );
    }

    #[test]
    fn texture_color_at_with_axis_inverted() {
        assert_eq!(
            apply_texture_color_at(AffineTransform {
                ux: 0.0,
                uy: 1.0,
                vx: 1.0,
                vy: 0.0,
                tx: 0.0,
                ty: 0.0
            }),
            [
                [0.1, 0.1, 0.1, 0.4, 0.4, 0.4, 0.4, 0.4],
                [0.2, 0.2, 0.2, 0.5, 0.5, 0.5, 0.5, 0.5],
                [0.3, 0.3, 0.3, 0.6, 0.6, 0.6, 0.6, 0.6],
                [0.1, 0.1, 0.1, 0.2, 0.2, 0.2, 0.2, 0.2],
            ],
        );
    }
}
