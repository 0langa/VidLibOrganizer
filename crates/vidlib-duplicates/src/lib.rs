use image::{imageops::FilterType, DynamicImage, GrayImage};
use blake3::Hasher as Blake3Hasher;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use vidlib_core::{DuplicateGroup, DuplicateStrategy, VideoEntry};

const CHUNK_SIZE: usize = 1024 * 1024;
const DCT_SIZE: usize = 32;
const PHASH_SIZE: usize = 8;

pub fn exact_hash(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn chunk_hash(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut hasher = Blake3Hasher::new();
    let mut buffer = vec![0_u8; CHUNK_SIZE.min(len.max(1) as usize)];

    for offset in sample_offsets(len) {
        file.seek(SeekFrom::Start(offset))?;
        let bytes_read = file.read(&mut buffer)?;
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

pub fn perceptual_hash_from_image_bytes(bytes: &[u8]) -> anyhow::Result<String> {
    let image = image::load_from_memory(bytes)?;
    perceptual_hash_image(&image)
}

pub fn perceptual_hash_image(image: &DynamicImage) -> anyhow::Result<String> {
    let gray = image
        .resize_exact(DCT_SIZE as u32, DCT_SIZE as u32, FilterType::Triangle)
        .to_luma8();
    let dct = dct_2d(&gray);
    let mut values = Vec::with_capacity(PHASH_SIZE * PHASH_SIZE - 1);
    for y in 0..PHASH_SIZE {
        for x in 0..PHASH_SIZE {
            if x == 0 && y == 0 {
                continue;
            }
            values.push(dct[y][x]);
        }
    }
    let mut sorted = values.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    let mut bits = String::with_capacity(PHASH_SIZE * PHASH_SIZE - 1);
    for value in values {
        bits.push(if value >= median { '1' } else { '0' });
    }
    Ok(bits)
}

pub fn hamming_distance(left: &str, right: &str) -> Option<u32> {
    if left.len() != right.len() {
        return None;
    }
    Some(
        left.bytes()
            .zip(right.bytes())
            .filter(|(a, b)| a != b)
            .count() as u32,
    )
}

fn sample_offsets(len: u64) -> Vec<u64> {
    if len <= CHUNK_SIZE as u64 {
        return vec![0];
    }
    let middle = len / 2;
    let end = len.saturating_sub(CHUNK_SIZE as u64);
    vec![0, middle, end]
}

pub fn group_duplicates(entries: &[VideoEntry]) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();
    groups.extend(group_by(entries, DuplicateStrategy::ExactHash, |e| {
        e.exact_hash.clone()
    }));
    groups.extend(group_by(entries, DuplicateStrategy::ChunkHash, |e| {
        e.chunk_hash.clone()
    }));
    groups.extend(group_by(entries, DuplicateStrategy::PerceptualHash, |e| {
        e.perceptual_hash.clone()
    }));
    groups.extend(group_by_perceptual_similarity(entries, 8));
    groups
}

fn group_by_perceptual_similarity(entries: &[VideoEntry], max_distance: u32) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();
    let mut used = vec![false; entries.len()];
    for i in 0..entries.len() {
        if used[i] {
            continue;
        }
        let Some(base) = entries[i].perceptual_hash.as_ref() else {
            continue;
        };
        let mut members = vec![entries[i].id];
        let mut total_size = entries[i].size_bytes;
        for j in (i + 1)..entries.len() {
            if used[j] {
                continue;
            }
            if let Some(candidate) = entries[j].perceptual_hash.as_ref() {
                if hamming_distance(base, candidate)
                    .map(|distance| distance <= max_distance)
                    .unwrap_or(false)
                {
                    used[j] = true;
                    members.push(entries[j].id);
                    total_size += entries[j].size_bytes;
                }
            }
        }
        if members.len() > 1 {
            groups.push(DuplicateGroup {
                strategy: DuplicateStrategy::PerceptualHash,
                fingerprint: base.clone(),
                members,
                total_size_bytes: total_size,
            });
        }
    }
    groups
}

fn dct_2d(gray: &GrayImage) -> [[f64; DCT_SIZE]; DCT_SIZE] {
    let mut output = [[0.0_f64; DCT_SIZE]; DCT_SIZE];
    let pixels: Vec<Vec<f64>> = (0..DCT_SIZE)
        .map(|y| {
            (0..DCT_SIZE)
                .map(|x| gray.get_pixel(x as u32, y as u32)[0] as f64)
                .collect()
        })
        .collect();
    for u in 0..DCT_SIZE {
        for v in 0..DCT_SIZE {
            let cu = if u == 0 { 1.0 / 2.0_f64.sqrt() } else { 1.0 };
            let cv = if v == 0 { 1.0 / 2.0_f64.sqrt() } else { 1.0 };
            let mut sum = 0.0;
            for (y, row) in pixels.iter().enumerate() {
                for (x, pixel) in row.iter().enumerate() {
                    sum += pixel
                        * ((std::f64::consts::PI * (2.0 * x as f64 + 1.0) * u as f64)
                            / (2.0 * DCT_SIZE as f64))
                            .cos()
                        * ((std::f64::consts::PI * (2.0 * y as f64 + 1.0) * v as f64)
                            / (2.0 * DCT_SIZE as f64))
                            .cos();
                }
            }
            output[v][u] = 0.25 * cu * cv * sum;
        }
    }
    output
}

fn group_by<F>(
    entries: &[VideoEntry],
    strategy: DuplicateStrategy,
    key_fn: F,
) -> Vec<DuplicateGroup>
where
    F: Fn(&VideoEntry) -> Option<String>,
{
    let mut map: HashMap<String, Vec<&VideoEntry>> = HashMap::new();
    for entry in entries {
        if let Some(key) = key_fn(entry) {
            map.entry(key).or_default().push(entry);
        }
    }
    map.into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(fingerprint, members)| DuplicateGroup {
            strategy: strategy.clone(),
            fingerprint,
            total_size_bytes: members.iter().map(|entry| entry.size_bytes).sum(),
            members: members.into_iter().map(|entry| entry.id).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, Luma};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn sample_entry(hash: &str) -> VideoEntry {
        VideoEntry {
            id: Uuid::new_v4(),
            path: PathBuf::from(hash),
            file_name: hash.into(),
            extension: Some("mp4".into()),
            size_bytes: 10,
            modified_at: None,
            duration_seconds: None,
            width: None,
            height: None,
            codec: None,
            exact_hash: Some(hash.into()),
            chunk_hash: None,
            perceptual_hash: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn detects_exact_hash_duplicates() {
        let entries = vec![
            sample_entry("same"),
            sample_entry("same"),
            sample_entry("other"),
        ];
        let groups = group_duplicates(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
    }

    #[test]
    fn computes_stable_perceptual_hash() {
        let mut image = GrayImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                image.put_pixel(x, y, Luma([if x < 16 { 10 } else { 220 }]));
            }
        }
        let hash = perceptual_hash_image(&DynamicImage::ImageLuma8(image)).unwrap();
        assert!(!hash.is_empty());
    }
}
