//! Display orientation of a source picture, applied when the compositor samples the clip (the
//! clip pass and its texture stay in stored orientation).

/// How stored pixels turn upright, as an EXIF orientation code: 1 (as stored) to 8. Video
/// rotation maps onto 1, 3, 6 and 8. Anything else is treated as 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Orient(pub u16);

impl Orient {
    fn code(self) -> u32 {
        if (1..=8).contains(&self.0) {
            self.0 as u32
        } else {
            1
        }
    }

    /// Displayed size of a stored `size` picture (codes 5..=8 swap the sides).
    pub fn size(self, (w, h): (u32, u32)) -> (u32, u32) {
        if self.code() >= 5 { (h, w) } else { (w, h) }
    }
}

/// The clip quad's uniform (`QuadU`) with the orientation code in the word after `mode`.
pub(crate) fn clip_quad(rect: [f32; 4], o: Orient) -> Vec<u8> {
    let mut u = super::quad_uniform(rect, [0.0; 4], 0);
    u[36..40].copy_from_slice(&o.code().to_le_bytes());
    u
}
