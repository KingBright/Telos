use std::collections::HashMap;

/// An Elementary Discourse Unit, which is the smallest piece of parsed text
#[derive(Debug, Clone)]
pub struct Edu {
    pub id: String,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// A pure Rust text chunker that splits text into EDUs by paragraphs and sentences.
/// This acts as a fallback for the Tree-sitter integration in a V1 MVP.
pub fn parse_into_edus(document_content: &str, base_id: &str) -> Vec<Edu> {
    let mut edus = Vec::new();
    let paragraphs: Vec<&str> = document_content.split("\n\n").filter(|p| !p.trim().is_empty()).collect();

    let mut counter = 0;
    for (p_idx, para) in paragraphs.iter().enumerate() {
        // Simplified sentence splitting based on punctuation.
        // For a more robust solution, we'd use something like the `unicode-segmentation` crate.
        let sentences: Vec<&str> = para.split(|c| c == '.' || c == '!' || c == '?')
            .filter(|s| !s.trim().is_empty())
            .collect();

        for (s_idx, sentence) in sentences.iter().enumerate() {
            let text = sentence.trim().to_string() + ".";
            edus.push(Edu {
                id: format!("{}_p{}_s{}_{}", base_id, p_idx, s_idx, counter),
                text,
                embedding: None,
            });
            counter += 1;
        }
    }

    edus
}

/// Calculates the cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for i in 0..a.len() {
        dot_product += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a.sqrt() * norm_b.sqrt())
}

/// A lightweight K-Means clustering algorithm in pure Rust.
/// Returns a map of cluster ID to a list of EDU IDs that belong to that cluster.
pub fn kmeans_cluster(edus: &[Edu], k: usize, max_iterations: usize) -> HashMap<usize, Vec<String>> {
    if edus.is_empty() || k == 0 {
        return HashMap::new();
    }

    // Ensure k is not larger than the number of EDUs
    let k = k.min(edus.len());
    let dim = edus[0].embedding.as_ref().map_or(0, |e| e.len());

    if dim == 0 {
        return HashMap::new();
    }

    // 1. Initialize centroids (randomly pick k EDUs for simplicity, but here we just take the first K)
    let mut centroids: Vec<Vec<f32>> = edus.iter().take(k).map(|e| e.embedding.clone().unwrap()).collect();

    let mut clusters: HashMap<usize, Vec<String>> = HashMap::new();

    for _ in 0..max_iterations {
        let mut new_clusters: HashMap<usize, Vec<String>> = HashMap::new();

        // 2. Assign points to the closest centroid
        for edu in edus {
            let emb = edu.embedding.as_ref().unwrap();
            let mut best_cluster = 0;
            // Since we are working with embeddings, we want to maximize cosine similarity
            let mut best_sim = -1.0;

            for (i, centroid) in centroids.iter().enumerate() {
                let sim = cosine_similarity(emb, centroid);
                if sim > best_sim {
                    best_sim = sim;
                    best_cluster = i;
                }
            }
            new_clusters.entry(best_cluster).or_insert_with(Vec::new).push(edu.id.clone());
        }

        // 3. Update centroids
        let mut moved = false;
        for (cluster_id, edu_ids) in &new_clusters {
            let mut new_centroid = vec![0.0; dim];
            for id in edu_ids {
                if let Some(edu) = edus.iter().find(|e| &e.id == id) {
                    let emb = edu.embedding.as_ref().unwrap();
                    for i in 0..dim {
                        new_centroid[i] += emb[i];
                    }
                }
            }

            // Average and normalize the new centroid
            let count = edu_ids.len() as f32;
            let mut norm = 0.0;
            for i in 0..dim {
                new_centroid[i] /= count;
                norm += new_centroid[i] * new_centroid[i];
            }
            norm = norm.sqrt();
            if norm > 0.0 {
                for i in 0..dim {
                    new_centroid[i] /= norm;
                }
            }

            // Check if centroid moved significantly (simple delta check)
            let sim = cosine_similarity(&centroids[*cluster_id], &new_centroid);
            if sim < 0.999 {
                moved = true;
                centroids[*cluster_id] = new_centroid;
            }
        }

        clusters = new_clusters;

        if !moved {
            break; // Converged
        }
    }

    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_into_edus() {
        let text = "This is a sentence. This is another! And a third?\n\nParagraph two.";
        let edus = parse_into_edus(text, "doc1");
        assert_eq!(edus.len(), 4);
        assert!(edus[0].text.starts_with("This is a sentence"));
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let c = vec![1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
        assert_eq!(cosine_similarity(&a, &c), 1.0);
    }
}
