use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::crypto::{aes_cbc_decrypt, blowfish_ecb, derive_entry_key};

pub const ENTRY_SIZE: usize = 528;
const HEADER_SIZE: usize = 8;
const TRAILER_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub label: &'static str,
    pub key: &'static str,
    pub base: &'static str,
}

pub const PROFILES: &[Profile] = &[
    Profile {
        label: "Steam (Global)",
        key: "83657ea6ffa1e671375c689a2e99a598",
        base: "1069d88738c5c75f82b44a1f0a382768",
    },
    Profile {
        label: "Russia",
        key: "a7f33db20dfb711a16d5d3dd3d4cef4d",
        base: "ee36ace0d87a9eaea565e6884a058b63",
    },
    Profile {
        label: "Japan",
        key: "c66e7c755883b5a92227b6c1c71c1d94",
        base: "51f912aba46f144bba8c3d6dcbf34728",
    },
    Profile {
        label: "Taiwan",
        key: "2d7ea2e4d31c53913fd2995ca031f777",
        base: "53f421141b5159fc5851ce2fd29c9e28",
    },
    Profile {
        label: "Korea",
        key: "287f1d85be26e55a1d994e9e1bfd0df1",
        base: "6ce3db2fbe338fba87edf2abf6453bfa",
    },
    Profile {
        label: "China",
        key: "a2e587d2fac8d6836cad3fd144f0cf2b",
        base: "c7db380e7d8d9e4771c7eaf5b1f3a78b",
    },
];

impl Default for Profile {
    fn default() -> Self {
        PROFILES[0]
    }
}

impl Profile {
    pub fn from_label(label: &str) -> Self {
        PROFILES
            .iter()
            .copied()
            .find(|profile| profile.label == label)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub size: u32,
    pub ensize: u32,
    pub zsize: u32,
    pub data: Vec<u8>,
}

impl Entry {
    pub fn display_path(&self) -> &str {
        self.name.trim_start_matches(['\\', '/'])
    }
}

#[derive(Clone, Debug, Default)]
pub struct Archive {
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct BuildSummary {
    pub entries: usize,
    pub bytes: u64,
}

pub struct PackageEntry {
    pub name: String,
    pub data: Vec<u8>,
}

struct BuiltEntry {
    name: String,
    size: u32,
    ensize: u32,
    zsize: u32,
    data: Vec<u8>,
}

impl Archive {
    pub fn read(path: &Path, profile: Profile) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("Could not read archive: {}", path.display()))?;
        Self::from_bytes(&data, profile)
    }

    pub fn from_bytes(data: &[u8], profile: Profile) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            bail!("This file is too small to be an LPK archive.");
        }

        let count = read_u32(data, 0)? as usize;
        if count == 0 {
            bail!("The archive contains no entries.");
        }

        let table_len = count
            .checked_mul(ENTRY_SIZE)
            .context("Invalid entry count in archive header.")?;
        let table_end = 4 + table_len;
        let data_start = table_end + 4;
        if data_start > data.len() {
            bail!("The archive header is truncated. Check the selected game profile.");
        }

        let table = blowfish_ecb(&data[4..table_end], profile.key.as_bytes(), false)
            .context("Could not decrypt the archive table. The selected profile may be wrong.")?;

        let mut entries = Vec::with_capacity(count);
        let mut offset = data_start as u64;

        for index in 0..count {
            let raw = &table[index * ENTRY_SIZE..(index + 1) * ENTRY_SIZE];
            let name_len = read_u32(raw, 0)? as usize;
            let max_name_len = ENTRY_SIZE - 4 - TRAILER_SIZE;
            if name_len == 0 || name_len > max_name_len {
                bail!("Entry {index} has an invalid name length.");
            }

            let name = std::str::from_utf8(&raw[4..4 + name_len])
                .with_context(|| format!("Entry {index} has a non-UTF-8 name."))?
                .to_string();

            let trailer = ENTRY_SIZE - TRAILER_SIZE;
            let size = read_u32(raw, trailer + 4)?;
            let ensize = read_u32(raw, trailer + 8)?;
            let zsize = read_u32(raw, trailer + 12)?;

            let end = offset + ensize as u64;
            if end > data.len() as u64 {
                bail!("Entry {index} points outside the archive.");
            }

            entries.push(Entry {
                name,
                size,
                ensize,
                zsize,
                data: data[offset as usize..end as usize].to_vec(),
            });
            offset = end;
        }

        Ok(Self { entries })
    }

    pub fn extract(&self, entry: &Entry, profile: Profile) -> Result<Vec<u8>> {
        if entry.zsize == 0 {
            let key = derive_entry_key(&entry.name, profile.base)?;
            let plain = aes_cbc_decrypt(&entry.data, &key)
                .with_context(|| format!("Could not decrypt {}", entry.name))?;
            if entry.size as usize > plain.len() {
                bail!("{} is smaller than its recorded size.", entry.name);
            }
            Ok(plain[..entry.size as usize].to_vec())
        } else {
            let zsize = entry.zsize as usize;
            if zsize > entry.data.len() {
                bail!("{} has an invalid compressed size.", entry.name);
            }
            let plain = blowfish_ecb(&entry.data, profile.key.as_bytes(), false)
                .with_context(|| format!("Could not decrypt {}", entry.name))?;
            let mut decoder = ZlibDecoder::new(&plain[..zsize]);
            let mut output = Vec::with_capacity(entry.size as usize);
            decoder
                .read_to_end(&mut output)
                .with_context(|| format!("Could not decompress {}", entry.name))?;
            if output.len() != entry.size as usize {
                bail!("{} decompressed to an unexpected size.", entry.name);
            }
            Ok(output)
        }
    }

    pub fn extract_all<F: FnMut(&str)>(
        &self,
        destination: &Path,
        profile: Profile,
        mut log: F,
    ) -> Result<usize> {
        let mut count = 0;

        for entry in &self.entries {
            let relative = safe_relative_path(&entry.name)?;
            let output = destination.join(relative);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Could not create {}", parent.display()))?;
            }
            let content = self.extract(entry, profile)?;
            fs::write(&output, content)
                .with_context(|| format!("Could not write {}", output.display()))?;
            log(&format!("Extracted {}", entry.display_path()));
            count += 1;
        }

        Ok(count)
    }
}

pub fn build_package<F: FnMut(&str)>(
    profile: Profile,
    entries: &[PackageEntry],
    destination: &Path,
    mut log: F,
) -> Result<BuildSummary> {
    if entries.is_empty() {
        bail!("The package has no files.");
    }

    let mut seen = HashSet::new();
    let mut built = Vec::with_capacity(entries.len());

    for entry in entries {
        safe_relative_path(&entry.name)?;
        if !seen.insert(normalize(&entry.name)) {
            bail!("Duplicate archive path: {}", entry.name);
        }

        let (size, ensize, zsize, data) = encode_entry(&entry.data, profile)?;
        log(&format!(
            "Packed {} ({} bytes)",
            entry.name,
            entry.data.len()
        ));
        built.push(BuiltEntry {
            name: entry.name.clone(),
            size,
            ensize,
            zsize,
            data,
        });
    }

    let bytes = write_archive(&built, profile, destination)?;
    Ok(BuildSummary {
        entries: built.len(),
        bytes,
    })
}

fn write_archive(built: &[BuiltEntry], profile: Profile, destination: &Path) -> Result<u64> {
    let mut table = vec![0u8; built.len() * ENTRY_SIZE];

    for (index, entry) in built.iter().enumerate() {
        let raw = &mut table[index * ENTRY_SIZE..(index + 1) * ENTRY_SIZE];
        let name = entry.name.as_bytes();
        if name.len() > ENTRY_SIZE - 4 - TRAILER_SIZE {
            bail!("Archive entry name is too long: {}", entry.name);
        }

        raw[..4].copy_from_slice(&(name.len() as u32).to_le_bytes());
        raw[4..4 + name.len()].copy_from_slice(name);

        let trailer = ENTRY_SIZE - TRAILER_SIZE;
        raw[trailer..trailer + 4].copy_from_slice(&0u32.to_le_bytes());
        raw[trailer + 4..trailer + 8].copy_from_slice(&entry.size.to_le_bytes());
        raw[trailer + 8..trailer + 12].copy_from_slice(&entry.ensize.to_le_bytes());
        raw[trailer + 12..trailer + 16].copy_from_slice(&entry.zsize.to_le_bytes());
    }

    let encrypted_table = blowfish_ecb(&table, profile.key.as_bytes(), true)?;
    let data_size: usize = built.iter().map(|entry| entry.ensize as usize).sum();
    let mut output = Vec::with_capacity(HEADER_SIZE + encrypted_table.len() + data_size);
    output.extend_from_slice(&(built.len() as u32).to_le_bytes());
    output.extend_from_slice(&encrypted_table);
    output.extend_from_slice(&0u32.to_le_bytes());
    for entry in built {
        output.extend_from_slice(&entry.data);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    fs::write(destination, &output)
        .with_context(|| format!("Could not write {}", destination.display()))?;

    Ok(output.len() as u64)
}

fn encode_entry(content: &[u8], profile: Profile) -> Result<(u32, u32, u32, Vec<u8>)> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content)?;
    let compressed = encoder.finish()?;
    let zsize = compressed.len() as u32;
    let ensize = compressed.len().div_ceil(8) * 8;

    let mut padded = compressed;
    padded.resize(ensize, 0);
    let encrypted = blowfish_ecb(&padded, profile.key.as_bytes(), true)?;

    Ok((content.len() as u32, ensize as u32, zsize, encrypted))
}

fn safe_relative_path(name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::new();

    for part in name.split(['\\', '/']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            bail!("Archive contains an unsafe path: {name}");
        }
        path.push(part);
    }

    if path.as_os_str().is_empty() {
        bail!("Archive entry has an empty path: {name}");
    }
    Ok(path)
}

pub fn normalize_path(name: &str) -> String {
    normalize(name)
}

fn normalize(name: &str) -> String {
    name.replace('/', "\\")
        .trim_matches('\\')
        .to_ascii_lowercase()
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .with_context(|| format!("Unexpected end of archive at offset {offset}."))?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4-byte slice")))
}

#[cfg(test)]
#[path = "../tests/unit/lpk.rs"]
mod tests;
