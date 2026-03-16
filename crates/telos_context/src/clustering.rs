use std::collections::HashMap;

/// An Elementary Discourse Unit, which is the smallest piece of parsed text
#[derive(Debug, Clone)]
pub struct Edu {
    pub id: String,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// A pure Rust text chunker that splits text into EDUs by paragraphs and sentences.
/// Automatically detects code content and routes to the structural code parser
/// for function/class-level splitting instead of sentence-level splitting.
pub fn parse_into_edus(document_content: &str, base_id: &str) -> Vec<Edu> {
    // Auto-detect: if content looks like code, use the structural code parser
    if crate::ast_parser::is_code_content(document_content) {
        return crate::ast_parser::parse_code_into_edus(document_content, base_id);
    }

    let mut edus = Vec::new();
    let paragraphs: Vec<&str> = document_content.split("\n\n").filter(|p| !p.trim().is_empty()).collect();

    let mut counter = 0;
    for (p_idx, para) in paragraphs.iter().enumerate() {
        // Simplified sentence splitting based on punctuation.
        // For a more robust solution, we'd use something like the `unicode-segmentation` crate.
        let sentences: Vec<&str> = para.split(['.', '!', '?'])
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
            new_clusters.entry(best_cluster).or_default().push(edu.id.clone());
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
            for val in new_centroid.iter_mut().take(dim) {
                *val /= count;
                norm += *val * *val;
            }
            norm = norm.sqrt();
            if norm > 0.0 {
                for val in new_centroid.iter_mut().take(dim) {
                *val /= norm;
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

/// Gaussian Mixture Model (GMM) soft clustering using the EM algorithm.
/// Unlike K-Means, each EDU can belong to *multiple* clusters with different
/// responsibilities (posterior probabilities). This preserves cross-cluster
/// information that hard clustering would lose.
///
/// Returns a map of cluster ID to list of (EDU ID, responsibility weight).
/// EDUs with responsibility > `threshold` are included in the cluster.
pub fn gmm_soft_cluster(
    edus: &[Edu],
    k: usize,
    max_iterations: usize,
    threshold: f32,
) -> HashMap<usize, Vec<(String, f32)>> {
    if edus.is_empty() || k == 0 {
        return HashMap::new();
    }

    let k = k.min(edus.len());
    let dim = edus[0].embedding.as_ref().map_or(0, |e| e.len());
    if dim == 0 {
        return HashMap::new();
    }

    // Collect embeddings as references for fast access
    let embeddings: Vec<&Vec<f32>> = edus.iter()
        .filter_map(|e| e.embedding.as_ref())
        .collect();
    let n = embeddings.len();
    if n < k {
        return HashMap::new();
    }

    // Initialize: means from first k EDUs, uniform weights, unit variance
    let mut means: Vec<Vec<f32>> = embeddings.iter().take(k).map(|e| (*e).clone()).collect();
    let mut weights: Vec<f32> = vec![1.0 / k as f32; k];
    // Diagonal covariance (variance per dimension per cluster) — avoids O(d^2) full covariance
    let mut variances: Vec<Vec<f32>> = vec![vec![1.0; dim]; k];

    // Responsibility matrix: r[i][j] = P(cluster j | point i)
    let mut responsibilities = vec![vec![0.0f32; k]; n];

    for _iter in 0..max_iterations {
        // === E-step: compute responsibilities ===
        for i in 0..n {
            let emb = embeddings[i];
            let mut log_probs = vec![0.0f64; k];

            for j in 0..k {
                // Log Gaussian with diagonal covariance:
                // log p(x|j) = -0.5 * sum_d [ (x_d - mu_d)^2 / var_d + ln(var_d) ] + ln(weight_j)
                let mut log_p: f64 = (weights[j] as f64).ln();
                for d in 0..dim {
                    let diff = emb[d] - means[j][d];
                    let var = variances[j][d].max(1e-6); // floor to avoid division by zero
                    log_p -= 0.5 * ((diff * diff) as f64 / var as f64 + (var as f64).ln());
                }
                log_probs[j] = log_p;
            }

            // Log-sum-exp for numerical stability
            let max_log = log_probs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let sum_exp: f64 = log_probs.iter().map(|lp| (lp - max_log).exp()).sum();
            let log_norm = max_log + sum_exp.ln();

            for j in 0..k {
                responsibilities[i][j] = ((log_probs[j] - log_norm).exp()) as f32;
            }
        }

        // === M-step: update means, variances, weights ===
        let mut converged = true;
        for j in 0..k {
            let n_j: f32 = responsibilities.iter().map(|r| r[j]).sum();
            if n_j < 1e-8 {
                continue; // dead cluster
            }

            // Update weight
            weights[j] = n_j / n as f32;

            // Update mean
            let mut new_mean = vec![0.0f32; dim];
            for i in 0..n {
                let r = responsibilities[i][j];
                for d in 0..dim {
                    new_mean[d] += r * embeddings[i][d];
                }
            }
            for d in 0..dim {
                new_mean[d] /= n_j;
            }

            // Check convergence (mean shift)
            let shift: f32 = new_mean.iter().zip(means[j].iter())
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                .sqrt();
            if shift > 1e-4 {
                converged = false;
            }

            // Update variance (diagonal)
            let mut new_var = vec![0.0f32; dim];
            for i in 0..n {
                let r = responsibilities[i][j];
                for d in 0..dim {
                    let diff = embeddings[i][d] - new_mean[d];
                    new_var[d] += r * diff * diff;
                }
            }
            for d in 0..dim {
                new_var[d] = (new_var[d] / n_j).max(1e-6); // floor variance
            }

            means[j] = new_mean;
            variances[j] = new_var;
        }

        if converged {
            break;
        }
    }

    // Build soft cluster assignments: include EDU if responsibility > threshold
    let mut clusters: HashMap<usize, Vec<(String, f32)>> = HashMap::new();
    for i in 0..n {
        for j in 0..k {
            if responsibilities[i][j] > threshold {
                clusters.entry(j)
                    .or_default()
                    .push((edus[i].id.clone(), responsibilities[i][j]));
            }
        }
    }

    // Sort each cluster by descending responsibility
    for members in clusters.values_mut() {
        members.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
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
