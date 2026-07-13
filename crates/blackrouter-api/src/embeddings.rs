//! Embedding + semantic recall primitives (Phase 8.3).
//!
//! The default embedder is a dependency-free, deterministic *lexical* embedder
//! based on the hashing trick. It is not a neural semantic model, but it gives
//! meaningful cosine similarity for overlapping vocabulary and requires no
//! external service. A production deployment can implement [`Embedder`] against
//! a real embeddings endpoint (e.g. OpenAI `/v1/embeddings`) and swap it in via
//! [`build_embedder`].

use std::hash::{Hash, Hasher};

/// Produce an embedding vector for a piece of text.
pub trait Embedder {
    /// Vector dimension produced by this embedder.
    fn dim(&self) -> usize;
    /// Embed `text` into a vector of length [`Embedder::dim`].
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Deterministic lexical embedder using the hashing trick over word tokens.
///
/// Each token is hashed to a bucket; the bucket accumulates a signed count
/// (`+1` / `-1` chosen by a second hash) so that cosine similarity correlates
/// with lexical overlap. The resulting vector is L2-normalized.
pub struct LocalLexicalEmbedder {
    dim: usize,
}

impl LocalLexicalEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }
}

impl Embedder for LocalLexicalEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0_f32; self.dim];
        for token in tokenize(text) {
            let idx = hash_token(&token, 0) % self.dim;
            let sign = if hash_token(&token, 1) & 1 == 1 {
                1.0_f32
            } else {
                -1.0_f32
            };
            vec[idx] += sign;
        }
        normalize(&mut vec);
        vec
    }
}

/// Build the configured embedder. Only the local lexical embedder ships today;
/// `kind` is accepted for forward-compatibility (a future `"remote"` neural
/// embedder would be selected here). The remote upgrade path is intentionally
/// not wired to a network call so the project stays dependency-free and
/// provider-agnostic.
pub fn build_embedder(_kind: &str, dim: usize) -> Box<dyn Embedder> {
    Box::new(LocalLexicalEmbedder::new(dim))
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn hash_token(token: &str, salt: u64) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    salt.hash(&mut hasher);
    hasher.finish() as usize
}

fn normalize(vec: &mut [f32]) {
    let magnitude: f32 = vec.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0_f32 {
        for value in vec.iter_mut() {
            *value /= magnitude;
        }
    }
}

/// Cosine similarity between two equal-length vectors. Returns `0.0` for empty
/// or mismatched vectors, or when either vector has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        mag_a += x * x;
        mag_b += y * y;
    }
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a.sqrt() * mag_b.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_embedder_is_deterministic_and_normalized() {
        let embedder = LocalLexicalEmbedder::new(64);
        let first = embedder.embed("the cat sat on the mat");
        let second = embedder.embed("the cat sat on the mat");
        assert_eq!(first, second);
        let magnitude: f32 = first.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((magnitude - 1.0_f32).abs() < 1e-5_f32);
    }

    #[test]
    fn similar_text_scores_higher_than_dissimilar() {
        let embedder = LocalLexicalEmbedder::new(128);
        let base = embedder.embed("rust programming language memory safety");
        let similar = embedder.embed("rust is a systems programming language");
        let different = embedder.embed("banana smoothie recipe healthy breakfast");
        let similar_score = cosine_similarity(&base, &similar);
        let different_score = cosine_similarity(&base, &different);
        assert!(
            similar_score > different_score,
            "similar {similar_score} should exceed different {different_score}"
        );
    }
}
