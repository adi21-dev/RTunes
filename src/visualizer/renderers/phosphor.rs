//! Phosphor persistence buffer for oscilloscope-style trails.

/// Row-major buffer of intensities in `[0, 1]` (terminal cell resolution).
pub struct PhosphorBuffer {
    width: u16,
    height: u16,
    cells: Vec<f32>,
    decay: f32,
}

impl PhosphorBuffer {
    pub fn new(decay: f32) -> Self {
        Self {
            width: 0,
            height: 0,
            cells: Vec::new(),
            decay,
        }
    }

    pub fn ensure_size(&mut self, w: u16, h: u16) {
        if w == self.width && h == self.height && !self.cells.is_empty() {
            return;
        }
        self.width = w;
        self.height = h;
        let len = usize::from(w) * usize::from(h);
        self.cells.clear();
        self.cells.resize(len, 0.0);
    }

    pub fn decay(&mut self) {
        let d = self.decay;
        for c in &mut self.cells {
            *c *= d;
        }
    }

    /// Multiply every cell by `factor` (e.g. `0.6` for temporal supersampling).
    pub fn scale_all(&mut self, factor: f32) {
        let f = factor.clamp(0.0, 1.0);
        for c in &mut self.cells {
            *c *= f;
        }
    }

    /// Saturating max at the given cell (clamped to buffer).
    pub fn paint(&mut self, x: i32, y: i32, intensity: f32) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let xi = x.clamp(0, i32::from(self.width) - 1) as usize;
        let yi = y.clamp(0, i32::from(self.height) - 1) as usize;
        let idx = yi * usize::from(self.width) + xi;
        if let Some(c) = self.cells.get_mut(idx) {
            *c = (*c).max(intensity);
        }
    }

    /// Iterate `(x, y, intensity)` for all cells above `threshold`.
    pub fn iter_lit(&self, threshold: f32) -> impl Iterator<Item = (u16, u16, f32)> + '_ {
        self.cells.iter().enumerate().filter_map(move |(idx, &v)| {
            if v <= threshold {
                return None;
            }
            let w = usize::from(self.width);
            if w == 0 {
                return None;
            }
            let y = (idx / w) as u16;
            let x = (idx % w) as u16;
            Some((x, y, v))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phosphor_decay_multiplies() {
        let mut p = PhosphorBuffer::new(0.85);
        p.ensure_size(10, 10);
        p.paint(5, 5, 1.0);
        let v0 = p
            .iter_lit(0.0)
            .find(|(x, y, _)| *x == 5 && *y == 5)
            .unwrap()
            .2;
        assert!((v0 - 1.0).abs() < 1e-5);
        p.decay();
        let v1 = p
            .iter_lit(0.0)
            .find(|(x, y, _)| *x == 5 && *y == 5)
            .unwrap()
            .2;
        assert!((v1 - 0.85).abs() < 1e-5);
    }
}
