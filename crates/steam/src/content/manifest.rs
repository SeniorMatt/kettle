//! Depot manifests: the zip from the CDN holds one binary file made of
//! length-prefixed protobuf sections (payload, metadata, signature).

use super::crypto::{self, DepotKey};
use anyhow::{Context, Result, bail, ensure};
use steam_vent_proto_common::protobuf::Message;
use steam_vent_proto_steam::content_manifest::{ContentManifestMetadata, ContentManifestPayload};

const MAGIC_PAYLOAD: u32 = 0x71F6_17D0;
const MAGIC_METADATA: u32 = 0x1F48_12BE;
const MAGIC_SIGNATURE: u32 = 0x1B81_B817;
const MAGIC_END: u32 = 0x32C4_15AB;

// EDepotFileFlag
const FLAG_EXECUTABLE: u32 = 32;
const FLAG_DIRECTORY: u32 = 64;

#[derive(Debug, Clone)]
pub struct Chunk {
    /// SHA-1 of the uncompressed data; also the chunk's name on the CDN.
    pub sha: [u8; 20],
    /// Steam's Adler-style checksum of the uncompressed data.
    pub crc: u32,
    pub offset: u64,
    pub size: u32,
    pub compressed_size: u32,
}

impl Chunk {
    pub fn id(&self) -> String {
        hex(&self.sha)
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Forward-slash relative path, checked to stay inside the install folder.
    pub path: String,
    pub size: u64,
    pub flags: u32,
    pub sha_content: Vec<u8>,
    pub chunks: Vec<Chunk>,
    pub link: Option<String>,
}

impl FileEntry {
    pub fn is_dir(&self) -> bool {
        self.flags & FLAG_DIRECTORY != 0
    }
    pub fn is_exec(&self) -> bool {
        self.flags & FLAG_EXECUTABLE != 0
    }
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub depot: u32,
    pub gid: u64,
    pub created: u32,
    pub files: Vec<FileEntry>,
    /// Uncompressed size of all files as the manifest metadata states it.
    pub total_size: u64,
    /// The decompressed manifest file as served, for the depot cache.
    pub raw: Vec<u8>,
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Steam's chunk checksum: Adler-32 with both sums starting at zero.
pub fn checksum(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (0u32, 0u32);
    // 5552 is the largest run before `b` could overflow a u32.
    for run in data.chunks(5552) {
        for &x in run {
            a += u32::from(x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Unzip a manifest download and parse it.
pub fn from_zip(zip: &[u8], key: &DepotKey) -> Result<Manifest> {
    parse(super::compress::unzip_first(zip).context("manifest zip")?, key)
}

/// Parse the sections of a decompressed manifest file.
pub fn parse(raw: Vec<u8>, key: &DepotKey) -> Result<Manifest> {
    let (mut payload, mut meta, mut signature) = (None, None, Vec::new());
    let mut pos = 0;
    while pos + 4 <= raw.len() {
        let magic = u32::from_le_bytes(raw[pos..pos + 4].try_into()?);
        pos += 4;
        if magic == MAGIC_END {
            break;
        }
        ensure!(pos + 4 <= raw.len(), "manifest truncated");
        let len = u32::from_le_bytes(raw[pos..pos + 4].try_into()?) as usize;
        pos += 4;
        let body = raw.get(pos..pos + len).context("manifest section truncated")?;
        pos += len;
        match magic {
            MAGIC_PAYLOAD => payload = Some(ContentManifestPayload::parse_from_bytes(body)?),
            MAGIC_METADATA => meta = Some(ContentManifestMetadata::parse_from_bytes(body)?),
            MAGIC_SIGNATURE => signature = body.to_vec(),
            other => bail!("unknown manifest section {other:#x}"),
        }
    }
    let (mut payload, mut meta) =
        (payload.context("manifest has no payload")?, meta.context("manifest has no metadata")?);
    let encrypted = meta.filenames_encrypted();
    let mut files = Vec::with_capacity(payload.mappings.len());
    for m in &mut payload.mappings {
        if encrypted {
            // Like Steam's own depotcache files, keep the clear names so the cache needs no key.
            let clear = crypto::decrypt_filename_raw(key, m.filename())?;
            m.set_filename(clear);
        }
        files.push(FileEntry {
            path: safe_path(&m.filename().replace('\\', "/"))?,
            size: m.size(),
            flags: m.flags(),
            sha_content: m.sha_content().to_vec(),
            chunks: m.chunks.iter().map(chunk).collect::<Result<_>>()?,
            link: m.linktarget.clone().filter(|l| !l.is_empty()),
        });
    }
    meta.set_filenames_encrypted(false);
    let raw = encode(&payload, &meta, &signature)?;
    Ok(Manifest {
        depot: meta.depot_id(),
        gid: meta.gid_manifest(),
        created: meta.creation_time(),
        files,
        total_size: meta.cb_disk_original(),
        raw,
    })
}

/// Serialize sections in the order Steam writes them.
fn encode(payload: &ContentManifestPayload, meta: &ContentManifestMetadata, signature: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (magic, body) in [
        (MAGIC_PAYLOAD, payload.write_to_bytes()?),
        (MAGIC_METADATA, meta.write_to_bytes()?),
        (MAGIC_SIGNATURE, signature.to_vec()),
    ] {
        out.extend_from_slice(&magic.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
    }
    out.extend_from_slice(&MAGIC_END.to_le_bytes());
    Ok(out)
}

fn chunk(
    c: &steam_vent_proto_steam::content_manifest::content_manifest_payload::file_mapping::ChunkData,
) -> Result<Chunk> {
    Ok(Chunk {
        sha: c.sha().try_into().map_err(|_| anyhow::anyhow!("chunk id is not 20 bytes"))?,
        crc: c.crc(),
        offset: c.offset(),
        size: c.cb_original(),
        compressed_size: c.cb_compressed(),
    })
}

/// A manifest is data from the network: never let it name a path outside the
/// install folder.
pub fn safe_path(p: &str) -> Result<String> {
    let p = p.trim_start_matches('/');
    ensure!(!p.is_empty(), "empty file name in manifest");
    for part in p.split('/') {
        ensure!(part != ".." && !part.contains('\0'), "unsafe path in manifest: {p:?}");
    }
    ensure!(!p.contains(':') || !p.split('/').next().unwrap_or("").ends_with(':'), "unsafe path in manifest: {p:?}");
    Ok(p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_vent_proto_steam::content_manifest::content_manifest_payload::{FileMapping, file_mapping::ChunkData};

    #[test]
    fn checksum_known_vectors() {
        // Standard Adler-32 of "Wikipedia" is 0x11E60398 (sums start at 1); starting at 0 loses exactly the
        // contribution of the initial 1: a-1 and b-9 (one per byte).
        assert_eq!(checksum(b""), 0);
        assert_eq!(checksum(b"Wikipedia"), 0x11E6_0398u32.wrapping_sub(9 << 16).wrapping_sub(1));
        let big = vec![0xFFu8; 100_000];
        let (mut a, mut b) = (0u64, 0u64);
        for &x in &big {
            a = (a + u64::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        assert_eq!(checksum(&big), ((b << 16) | a) as u32);
    }

    #[test]
    fn rejects_traversal() {
        assert!(safe_path("a/../../etc").is_err());
        assert_eq!(safe_path("/bin/x").unwrap(), "bin/x");
    }

    #[test]
    fn parses_encrypted_manifest() {
        use base64::Engine as _;
        let key: DepotKey = [5; 32];
        let name = base64::engine::general_purpose::STANDARD.encode(crypto::encrypt(&key, [1; 16], b"dir\\file.bin\0"));
        let mut cd = ChunkData::new();
        cd.set_sha(vec![0xAB; 20]);
        cd.set_crc(77);
        cd.set_offset(0);
        cd.set_cb_original(10);
        cd.set_cb_compressed(8);
        let mut fm = FileMapping::new();
        fm.set_filename(name);
        fm.set_size(10);
        fm.set_flags(32);
        fm.chunks.push(cd);
        let mut payload = ContentManifestPayload::new();
        payload.mappings.push(fm);
        let mut meta = ContentManifestMetadata::new();
        meta.set_depot_id(9);
        meta.set_gid_manifest(1234);
        meta.set_filenames_encrypted(true);
        meta.set_cb_disk_original(10);
        let mut raw = Vec::new();
        for (magic, body) in
            [(MAGIC_PAYLOAD, payload.write_to_bytes().unwrap()), (MAGIC_METADATA, meta.write_to_bytes().unwrap())]
        {
            raw.extend_from_slice(&magic.to_le_bytes());
            raw.extend_from_slice(&(body.len() as u32).to_le_bytes());
            raw.extend_from_slice(&body);
        }
        raw.extend_from_slice(&MAGIC_END.to_le_bytes());
        let zip = super::super::compress::tests::make_zip("z", &raw, true);
        let m = from_zip(&zip, &key).unwrap();
        assert_eq!((m.depot, m.gid, m.files.len()), (9, 1234, 1));
        assert_eq!(m.files[0].path, "dir/file.bin");
        assert!(m.files[0].is_exec() && !m.files[0].is_dir());
        assert_eq!(m.files[0].chunks[0].id(), "ab".repeat(20));
    }
}
