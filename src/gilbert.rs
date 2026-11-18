//! Generalized Hilbert ("gilbert") space-filling curve for rectangles of any size.
//!
//! Algorithm by Jakub Červený (https://github.com/jakubcerveny/gilbert), reimplemented in Rust.
//! The original is distributed under this license:
//!
//! BSD 2-Clause License
//!
//! Copyright (c) 2018, Jakub Červený
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are met:
//!
//! * Redistributions of source code must retain the above copyright notice, this
//!   list of conditions and the following disclaimer.
//!
//! * Redistributions in binary form must reproduce the above copyright notice,
//!   this list of conditions and the following disclaimer in the documentation
//!   and/or other materials provided with the distribution.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
//! AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
//! IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//! DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
//! FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
//! DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//! SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
//! CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
//! OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
//! OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

/// Calls `f(x, y)` for every cell of a `w`×`h` rectangle, in curve order.
pub fn curve<F: FnMut(i64, i64)>(w: i64, h: i64, f: &mut F) {
    if w <= 0 || h <= 0 {
        return;
    }
    if w >= h {
        generate(0, 0, w, 0, 0, h, f);
    } else {
        generate(0, 0, 0, h, w, 0, f);
    }
}

fn generate<F: FnMut(i64, i64)>(x: i64, y: i64, ax: i64, ay: i64, bx: i64, by: i64, f: &mut F) {
    let w = (ax + ay).abs();
    let h = (bx + by).abs();
    let (dax, day) = (ax.signum(), ay.signum());
    let (dbx, dby) = (bx.signum(), by.signum());

    if h == 1 {
        for i in 0..w {
            f(x + i * dax, y + i * day);
        }
        return;
    }
    if w == 1 {
        for i in 0..h {
            f(x + i * dbx, y + i * dby);
        }
        return;
    }

    // floor division, as in the original
    let (mut ax2, mut ay2) = (ax.div_euclid(2), ay.div_euclid(2));
    let (mut bx2, mut by2) = (bx.div_euclid(2), by.div_euclid(2));
    let w2 = (ax2 + ay2).abs();
    let h2 = (bx2 + by2).abs();

    if 2 * w > 3 * h {
        if w2 % 2 != 0 && w > 2 {
            ax2 += dax;
            ay2 += day;
        }
        // long case: split in two parts only
        generate(x, y, ax2, ay2, bx, by, f);
        generate(x + ax2, y + ay2, ax - ax2, ay - ay2, bx, by, f);
    } else {
        if h2 % 2 != 0 && h > 2 {
            bx2 += dbx;
            by2 += dby;
        }
        // standard case: one step up, one long horizontal, one step down
        generate(x, y, bx2, by2, ax2, ay2, f);
        generate(x + bx2, y + by2, ax, ay, bx - bx2, by - by2, f);
        generate(
            x + (ax - dax) + (bx2 - dbx),
            y + (ay - day) + (by2 - dby),
            -bx2,
            -by2,
            -(ax - ax2),
            -(ay - ay2),
            f,
        );
    }
}
