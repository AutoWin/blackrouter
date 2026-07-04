use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CompressionStats {
    pub original_bytes: usize,
    pub compressed_bytes: usize,
}

impl CompressionStats {
    pub fn saved_bytes(&self) -> usize {
        self.original_bytes.saturating_sub(self.compressed_bytes)
    }

    pub fn saved_ratio(&self) -> f64 {
        if self.original_bytes == 0 {
            return 0.0;
        }

        self.saved_bytes() as f64 / self.original_bytes as f64
    }
}
