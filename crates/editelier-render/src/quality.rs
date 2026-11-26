//! Image comparison for golden-frame and export tests.

/// Mean SSIM of two 8-bit single-channel images (`w * h` bytes each), 8×8 windows, stride 4,
/// standard constants (K1 = 0.01, K2 = 0.03, L = 255).
pub fn ssim(a: &[u8], b: &[u8], w: usize, h: usize) -> f64 {
    assert!(
        a.len() >= w * h && b.len() >= w * h,
        "image smaller than w*h"
    );
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let (mut sum, mut n) = (0.0, 0usize);
    for y0 in (0..h.saturating_sub(7)).step_by(4) {
        for x0 in (0..w.saturating_sub(7)).step_by(4) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in y0..y0 + 8 {
                for x in x0..x0 + 8 {
                    let (p, q) = (a[y * w + x] as f64, b[y * w + x] as f64);
                    sa += p;
                    sb += q;
                    saa += p * p;
                    sbb += q * q;
                    sab += p * q;
                }
            }
            let k = 64.0;
            let (ma, mb) = (sa / k, sb / k);
            let (va, vb) = (saa / k - ma * ma, sbb / k - mb * mb);
            let cov = sab / k - ma * mb;
            sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1;
        }
    }
    if n == 0 { 1.0 } else { sum / n as f64 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_is_one_and_noise_is_lower() {
        let a: Vec<u8> = (0..64 * 64).map(|i| (i % 251) as u8).collect();
        assert!((super::ssim(&a, &a, 64, 64) - 1.0).abs() < 1e-12);
        let b: Vec<u8> = a
            .iter()
            .enumerate()
            .map(|(i, &v)| v.wrapping_add(((i * 7919) % 61) as u8))
            .collect();
        assert!(super::ssim(&a, &b, 64, 64) < 0.97);
    }
}
