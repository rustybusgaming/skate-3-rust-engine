// Presents the scaled scene target in the window (`graphics_quality.rs`). Mode 1: edge-adaptive
// upscale below 100% internal resolution. Mode 2: box-filtered resolve above 100%
// (supersampling). Bilinear presentation, the default, never reaches this shader: it stays on
// the stock ImageNode.
//
// The edge-adaptive mode ports EASU from AMD FidelityFX Super Resolution 1.0 (ffx_fsr1.h,
// FsrEasuF and its helpers), keeping its operation order and its approximate reciprocals:
//
//   Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
//   Permission is hereby granted, free of charge, to any person obtaining a copy of this
//   software and associated documentation files (the "Software"), to deal in the Software
//   without restriction, including without limitation the rights to use, copy, modify, merge,
//   publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
//   to whom the Software is furnished to do so, subject to the following conditions:
//   The above copyright notice and this permission notice shall be included in all copies or
//   substantial portions of the Software.
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
//   INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
//   PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
//   FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//   OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//   DEALINGS IN THE SOFTWARE.
#import bevy_ui::ui_vertex_output::UiVertexOutput

struct Params {
    // Scene target and window sizes in pixels.
    source: vec2<f32>,
    output: vec2<f32>,
    mode: u32,
}

@group(1) @binding(0) var<uniform> params: Params;
@group(1) @binding(1) var source: texture_2d<f32>;
@group(1) @binding(2) var source_sampler: sampler;

const MODE_EDGE: u32 = 1u;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let alpha = textureSampleLevel(source, source_sampler, in.uv, 0.0).a;
    if params.mode == MODE_EDGE {
        return vec4(easu(in.uv), alpha);
    }
    return vec4(resolve(in.uv), alpha);
}

// Four bilinear taps a quarter of the pixel's footprint from its centre (0.25 / output in UV,
// whatever the ratio): an exact 2x2 box at 200%, a box blended with the bilinear tent between.
// Linear light, so the average is energy-correct.
fn resolve(uv: vec2<f32>) -> vec3<f32> {
    let d = 0.25 / params.output;
    let a = textureSampleLevel(source, source_sampler, uv + vec2(-d.x, -d.y), 0.0).rgb;
    let b = textureSampleLevel(source, source_sampler, uv + vec2(d.x, -d.y), 0.0).rgb;
    let c = textureSampleLevel(source, source_sampler, uv + vec2(-d.x, d.y), 0.0).rgb;
    let e = textureSampleLevel(source, source_sampler, uv + vec2(d.x, d.y), 0.0).rgb;
    return (a + b + c + e) * 0.25;
}

// APrxLoRcpF1 / APrxLoRsqF1: bit-trick approximations. They are finite at zero, which the
// direction analysis relies on for flat regions (0 * a huge finite value is 0, not NaN).
fn rcp_approx(x: f32) -> f32 {
    return bitcast<f32>(0x7ef07ebbu - bitcast<u32>(x));
}

fn rsq_approx(x: f32) -> f32 {
    return bitcast<f32>(0x5f347d74u - (bitcast<u32>(x) >> 1u));
}

// EASU wants perceptual input: the target holds linear light, so work in gamma 2.
fn texel(p: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(source)) - 1;
    return sqrt(textureLoad(source, clamp(p, vec2(0), last), 0).rgb);
}

// Luma times two in two multiply-adds, as EASU uses it.
fn luma(c: vec3<f32>) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

struct Analysis {
    dir: vec2<f32>,
    len: f32,
}

// FsrEasuSetF: one of the four bilinear corners' edge direction and length, from the '+' of
// lumas around it (a above, b left, c centre, d right, e below), weighted by `w`.
fn easu_set(acc: ptr<function, Analysis>, w: f32, la: f32, lb: f32, lc: f32, ld: f32, le: f32) {
    let dc = ld - lc;
    let cb = lc - lb;
    var len_x = rcp_approx(max(abs(dc), abs(cb)));
    let dir_x = ld - lb;
    (*acc).dir.x += dir_x * w;
    len_x = saturate(abs(dir_x) * len_x);
    len_x *= len_x;
    (*acc).len += len_x * w;
    let ec = le - lc;
    let ca = lc - la;
    var len_y = rcp_approx(max(abs(ec), abs(ca)));
    let dir_y = le - la;
    (*acc).dir.y += dir_y * w;
    len_y = saturate(abs(dir_y) * len_y);
    len_y *= len_y;
    (*acc).len += len_y * w;
}

// FsrEasuTapF: one texel's weight under the rotated, stretched Lanczos-2 approximation.
fn easu_tap(acc: ptr<function, vec4<f32>>, off: vec2<f32>, dir: vec2<f32>, len: vec2<f32>, lob: f32, clp: f32, c: vec3<f32>) {
    var v = vec2(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x);
    v *= len;
    let d2 = min(v.x * v.x + v.y * v.y, clp);
    //  (25/16 * (2/5 * x^2 - 1)^2 - (25/16 - 1)) * (1/4 * x^2 - 1)^2, with the window's lobe varying.
    var wb = (2.0 / 5.0) * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = (25.0 / 16.0) * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    *acc += vec4(c * w, w);
}

// FsrEasuF.
fn easu(uv: vec2<f32>) -> vec3<f32> {
    // 'f' is the top-left of the four texels around the sample; pp the fraction past it.
    let position = uv * params.source - 0.5;
    let fp = floor(position);
    let pp = position - fp;
    let at = vec2<i32>(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let b = texel(at + vec2(0, -1));
    let c = texel(at + vec2(1, -1));
    let e = texel(at + vec2(-1, 0));
    let f = texel(at);
    let g = texel(at + vec2(1, 0));
    let h = texel(at + vec2(2, 0));
    let i = texel(at + vec2(-1, 1));
    let j = texel(at + vec2(0, 1));
    let k = texel(at + vec2(1, 1));
    let l = texel(at + vec2(2, 1));
    let n = texel(at + vec2(0, 2));
    let o = texel(at + vec2(1, 2));
    let bl = luma(b);
    let cl = luma(c);
    let el = luma(e);
    let fl = luma(f);
    let gl = luma(g);
    let hl = luma(h);
    let il = luma(i);
    let jl = luma(j);
    let kl = luma(k);
    let ll = luma(l);
    let nl = luma(n);
    let ol = luma(o);

    var analysis = Analysis(vec2(0.0), 0.0);
    easu_set(&analysis, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    easu_set(&analysis, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    easu_set(&analysis, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    easu_set(&analysis, pp.x * pp.y, gl, jl, kl, ll, ol);

    // Normalise the direction, and clean up close to zero.
    var dir = analysis.dir;
    let dir2 = dir * dir;
    var dir_r = dir2.x + dir2.y;
    let zero = dir_r < 1.0 / 32768.0;
    dir_r = select(rsq_approx(dir_r), 1.0, zero);
    dir.x = select(dir.x, 1.0, zero);
    dir *= dir_r;
    // Length from {0..2} to {0..1}, shaped with a square.
    var len = analysis.len * 0.5;
    len *= len;
    // Stretch the kernel from 1 (axis-aligned) to sqrt(2) (diagonal), and narrow it across edges.
    let stretch = (dir.x * dir.x + dir.y * dir.y) * rcp_approx(max(abs(dir.x), abs(dir.y)));
    let len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    // The window widens from sqrt(2) to slightly beyond 2 with the amount of edge.
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = rcp_approx(lob);

    var sum = vec4(0.0);
    easu_tap(&sum, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, b);
    easu_tap(&sum, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, c);
    easu_tap(&sum, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
    easu_tap(&sum, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, j);
    easu_tap(&sum, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, f);
    easu_tap(&sum, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
    easu_tap(&sum, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, k);
    easu_tap(&sum, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, l);
    easu_tap(&sum, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, h);
    easu_tap(&sum, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, g);
    easu_tap(&sum, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, o);
    easu_tap(&sum, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, n);

    // Normalise, and dering against the four nearest texels.
    let min4 = min(min(f, g), min(j, k));
    let max4 = max(max(f, g), max(j, k));
    let color = min(max4, max(min4, sum.rgb / sum.w));
    return color * color;
}
